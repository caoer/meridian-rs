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
    /// The MOUNT's canonical name — never the alias the caller spelled. Every
    /// canonical echo (`resolve`'s `ref:` row, receipts, pins) is built from
    /// this, which is what keeps aliases out of stored form (§ 4.6a).
    pub(crate) name: addr::MountName,
    /// The root's canonical bound path — the workspace the read binds to.
    pub(crate) workspace: PathBuf,
    /// The mount's declared-primary designation, carried from the one table
    /// read so a door that renders the root row (resolve) never opens the
    /// table a second time for it.
    pub(crate) primary: bool,
    /// The alias the caller SPELLED, when the mount was reached by one —
    /// `None` when the spelling was the mount's own name. Carried so the
    /// resolve door can show which spelling landed where without opening the
    /// table again; never part of a canonical echo.
    pub(crate) alias: Option<addr::MountName>,
}

impl RootedRef {
    /// The CANONICAL rooted spelling of `rel` in this root — `name:rel`, never
    /// the alias the caller wrote (`address-grammar.md` § 4.6a).
    ///
    /// ONE owner for the string every door echoes back. Each door used to hold
    /// the caller's own bytes, which was correct while a rooted spelling could
    /// only BE the mount's name; aliases ended that, and a per-door `format!`
    /// is a rule spelled six times that drifts at five of them. A door that
    /// echoes a resolved rooted ref calls this.
    pub(crate) fn canonical_ref(&self, rel: &str) -> String {
        format!("{}:{rel}", self.name)
    }
}

/// Does this spelling enter the rooted lane at all? — the same lexical gate
/// the link plane and the resolver's C-3 guard share.
pub(crate) fn is_rooted(spelling: &str) -> bool {
    addr::head_carries_root_separator(spelling)
}

/// The alias half of the unbound-root refusal (`meridian-md-schema.md` § 5.1b).
///
/// A consumer that spells ONE constant — a skill writing `sessions:` — needs the
/// refusal to teach the one line that makes its constant resolve. "Declare the
/// mount" alone is the WRONG remedy when the tree is already mounted under
/// another name: it sends a reader to add a second mount for a tree they have.
///
/// At `sessions` this renders, verbatim:
///
/// ```text
/// declare `alias: sessions` on the mount that holds that tree
/// ```
///
/// Pinned by `tests/root_alias.rs::a_table_with_neither_refuses_and_teaches_the_alias_line`,
/// which asserts those bytes off a REAL refusal through the binary — produced,
/// not asserted (the house pattern, `address-grammar.md` § 6).
pub(crate) fn alias_teaching(name: &addr::MountName) -> String {
    format!("declare `alias: {name}` on the mount that holds that tree")
}

/// The peel-then-admit sequence, one call per door: `Ok(None)` for an ambient
/// spelling — the caller's own §1 admission still applies to it — and
/// `Ok(Some((rel, rooted)))` for a rooted spelling, resolved through
/// [`resolve`]. The error is the seam's `bad_path` family; the door frames it
/// in its own envelope (an address-plane refusal is exit 1, never exit 2).
///
/// Deliberately carries no fragment policy: whether a `#` is legal at a door
/// is that door's own stance (read splits it, resolve/put refuse it), so the
/// seam stays a pure de-duplication of the gate-then-resolve lines.
pub(crate) fn enter(
    spelling: &str,
    door: &str,
    consequence: &str,
) -> Result<Option<(String, RootedRef)>, Box<ErrorBody>> {
    if !is_rooted(spelling) {
        return Ok(None);
    }
    resolve(spelling, door, consequence).map(Some)
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
    let rooted = resolve_name(&name, spelling, consequence)?;
    Ok((rel, rooted))
}

/// The table half of [`resolve`]: a canonical root NAME to its bound
/// workspace — shared with the door surfaces that take a bare name instead
/// of a `root:path` spelling (`sql --root`). `spelling` is what the refusals
/// echo: the caller's own bytes, rooted ref or bare name alike.
///
/// # Errors
/// `bad_path` (§8) — an unreadable mount table, a root this machine does not
/// bind (the refusal enumerates what DOES bind), or a declared-but-unbound
/// root (the refusal carries the mount's own state word and detail).
pub(crate) fn resolve_name(
    name: &addr::MountName,
    spelling: &str,
    consequence: &str,
) -> Result<RootedRef, Box<ErrorBody>> {
    let refuse = |message: String| -> Box<ErrorBody> {
        let mut e = ErrorBody::new(ErrorCode::BadPath);
        e.path = Some(wire::Path(spelling.to_owned()));
        e.message = Some(message);
        Box::new(e)
    };
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
    // Name first, then alias (§ 5.1b) — the ONE lookup order, so `sessions:`
    // reaches a mount named `sessions` and a mount aliased `sessions` alike, and
    // nothing has to know which spelling this machine happens to use.
    let Some(mount) = table.by_name_or_alias(name.as_str()) else {
        let names: Vec<String> = table
            .mounts()
            .iter()
            .filter(|m| !m.state().refuses())
            .map(|m| match m.alias() {
                Some(alias) => format!("{} (alias {alias})", m.name()),
                None => m.name().to_owned(),
            })
            .collect();
        return Err(refuse(format!(
            "{spelling} names root `{name}`, which this machine does not bind (bound roots: \
             {}). {consequence} Fix: declare the mount in ~/MERIDIAN.md (name / path), or {} \
             — a root is looked up by name first and only then by alias, so the tree may \
             already be here under another name; see [[address-grammar]].",
            if names.is_empty() {
                "none".to_owned()
            } else {
                names.join(", ")
            },
            alias_teaching(name)
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
    // The MOUNT's name, not the caller's spelling: when an alias landed the
    // lookup, every canonical echo downstream must still print the name.
    let canonical = addr::MountName::parse(mount.name()).map_err(|e| {
        refuse(format!(
            "{spelling} resolves to a mount named `{}`, which is not a canonical root name \
             ({e}). {consequence}",
            mount.name()
        ))
    })?;
    let alias = (canonical != *name).then(|| name.clone());
    Ok(RootedRef {
        name: canonical,
        workspace: workspace.to_path_buf(),
        primary: mount.primary(),
        alias,
    })
}
