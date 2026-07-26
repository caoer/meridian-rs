//! `mrd config` — the config plane's publishing surface (U6).
//!
//! ```text
//! mrd config [--json]
//! ```
//!
//! Resolves the `MERIDIAN.md` bootstrap chain and prints what it found: the
//! resolved path, the state, the config's own rev and fingerprint, and the
//! declared mounts and tools in document order. Read-only — it writes nothing
//! and mints nothing.
//!
//! # Why this verb exists rather than an existing one
//! Two surfaces were MEASURED out before this one was written, not read out of
//! a `--help`:
//!
//! - **`mrd read`** routes through the render face, which ELIDES every
//!   `meridian-*` block (`TextRenderer::with_meridian_elision`). Measured on
//!   the installed binary: `mrd read MERIDIAN.md --section '…/Roots'` prints the
//!   prose, drops all three mount blocks, and exits 0. An agent reading it would
//!   reasonably report that the config failed to parse when it parsed perfectly
//!   — the silent-absence class. Pinned by
//!   `testsuite/tests/meridian_md.rs::the_render_face_elides_config_blocks_and_leaves_no_marker`.
//! - **`mrd resolve`** is the WORKSPACE-identity sense of the word: it maps a
//!   filesystem path to a workspace and a cache drawer, and prints no mount
//!   table (`docs/address-grammar.md` §13).
//!
//! So the verb that publishes the parsed mount table is this one, and the
//! elision it works around is named in its own output rather than left for an
//! operator to rediscover.
//!
//! Exits: **0** resolved (loaded or absent) / **1** the config refused (its
//! message, verbatim) / **2** bad invocation.

use serde_json::{Value, json};

use crate::{Fail, Format};

/// Run `mrd config [--json]`. A positional argument or an unknown flag is
/// already an exit-2 from the shared argument parser (`NO_PATH`).
///
/// # Errors
/// [`Fail`] exit 1 when the chain refuses — a malformed config, a stated path
/// that is not a readable regular file, or an unbuildable `$HOME`.
pub(crate) fn run(format: Format) -> Result<(), Fail> {
    // The refusal rides verbatim: it already names what is broken, where, that
    // nothing loaded, and the fix. Re-wording it here would be a second
    // spelling of the same fact.
    let resolution =
        config::resolve(&config::Env::from_process()).map_err(|e| Fail::findings(e.to_string()))?;

    match format {
        Format::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&to_json(&resolution)).expect("json")
            );
        }
        Format::Human => print!("{}", render_human(&resolution)),
    }
    Ok(())
}

fn state(resolution: &config::Resolution) -> &'static str {
    match resolution {
        config::Resolution::Absent { .. } => "absent",
        config::Resolution::Loaded(_) => "loaded",
    }
}

fn to_json(resolution: &config::Resolution) -> Value {
    json!({
        "path": resolution.path().display().to_string(),
        "state": state(resolution),
        "file_rev": resolution.file_rev(),
        "fingerprint": resolution.config().and_then(config::Config::fingerprint),
        "mounts": resolution.mounts().iter().map(|m| json!({
            "name": m.name,
            "path": m.path,
            "kind": m.kind.as_str(),
            "vault": m.vault,
            "pin": m.pin,
        })).collect::<Vec<_>>(),
        "tools": resolution.tools().iter().map(|t| json!({
            "name": t.name,
            "kind": t.kind,
            "config": t.config,
        })).collect::<Vec<_>>(),
    })
}

fn render_human(resolution: &config::Resolution) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();

    // The resolved path VERBATIM, which is the spelling a refusal from the same
    // chain carries. Abbreviating it to `~/MERIDIAN.md` here would give one
    // file two spellings across two faces of one verb.
    let _ = write!(
        out,
        "{}  {}",
        resolution.path().display(),
        state(resolution)
    );
    if let Some(rev) = resolution.file_rev() {
        let _ = write!(out, "  file_rev:{rev}");
    }
    if let Some(fp) = resolution.config().and_then(config::Config::fingerprint) {
        let _ = write!(out, "  fp:{fp}");
    }
    out.push('\n');

    if matches!(resolution, config::Resolution::Absent { .. }) {
        // State A is not an error and not a warning: every machine starts here.
        // Saying so is what keeps an operator from reading a bare empty table as
        // a failure.
        out.push_str("no config file — single-root behaviour, unchanged\n");
    }

    let _ = writeln!(out, "mounts ({}):", resolution.mounts().len());
    for m in resolution.mounts() {
        let _ = write!(out, "  {}  {}  {}", m.name, m.kind.as_str(), m.path);
        if let Some(vault) = &m.vault {
            let _ = write!(out, "  vault:{vault}");
        }
        if let Some(pin) = &m.pin {
            let _ = write!(out, "  pin:{pin}");
        }
        out.push('\n');
    }
    let _ = writeln!(out, "tools ({}):", resolution.tools().len());
    for t in resolution.tools() {
        let _ = writeln!(out, "  {}  {}", t.name, t.kind);
    }

    if !resolution.mounts().is_empty() || !resolution.tools().is_empty() {
        // S3-R10(a): the render face elides these blocks and leaves no marker,
        // so `mrd read` on this same file shows the prose and none of the above.
        // Naming it here is what stops the elision from reading as a parse
        // failure.
        out.push_str(
            "note: meridian-* blocks are elided by the render face, so `mrd read` on this file\n\
             \x20     shows its prose and none of the entries above. This verb publishes them.\n",
        );
    }
    out
}
