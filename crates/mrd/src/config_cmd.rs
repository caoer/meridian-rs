//! `mrd config` — the config plane's publishing surface (U6).
//!
//! ```text
//! mrd config [--json]
//! ```
//!
//! Resolves the `MERIDIAN.md` bootstrap chain and prints what it found: the
//! resolved path, the state, the config's own rev and fingerprint, the **bound
//! mount table** with each root's state, and the declared tools in document
//! order. Read-only — it writes nothing and mints nothing.
//!
//! # This is criterion 2's surface as well as criterion 1's (U7)
//! The mount table is the single authority for the three-way translation —
//! canonical root name ↔ Obsidian vault name ↔ local path — so this verb prints
//! all three legs per root plus the state binding decided, and it prints the
//! **canonical** path beside the declared one whenever the two differ. A verb
//! that showed only the declared spelling would hide exactly the symlink the
//! mount law exists to collapse.
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
//! Exits: **0** resolved and every root **bound** / **1** the config refused
//! (its message, verbatim) **or any root refuses** — grey and red alike, each
//! with its own reason word / **2** bad invocation.
//!
//! **Grey rides exit 1, and that is S3-R6, not a local choice.** The exit code
//! answers one question — *may this proceed?* — and an unseeable or drifted root
//! answers no, exactly as a refusal does. There is **no fourth exit code**; the
//! reason word carries the difference, in the human line and in `--json` alike.
//! The table is printed before the non-zero exit, because a refusal an operator
//! cannot read teaches nothing.

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
    // A bind refusal rides verbatim for the same reason a parse refusal does:
    // it already names the mount, the line, that nothing loaded, and the fix.
    let table = resolution
        .bind()
        .map_err(|e| Fail::findings(e.to_string()))?;

    // The bridge period's check (U9). It NEVER changes the exit code: a
    // divergence between an env var and the file is a note, because fail-loud
    // here would brick the CLI on every machine that exports the variable.
    let bridged = config::bridge::check(&config::bridge::BridgeEnv::from_process(), &table);

    match format {
        Format::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&to_json(&resolution, &table, &bridged))
                    .expect("json")
            );
        }
        Format::Human => print!("{}", render_human(&resolution, &table, &bridged)),
    }

    if table.is_clear() {
        return Ok(());
    }
    // Printed first, then refused: the reason words are above, and this line
    // names which roots carry them so the exit is not a bare 1.
    let refusing: Vec<String> = table
        .mounts()
        .iter()
        .filter(|m| m.state().refuses())
        .map(|m| format!("{} {}", m.name(), m.state().word()))
        .collect();
    Err(Fail::findings(format!(
        "{} of {} roots refuse: {}",
        refusing.len(),
        table.mounts().len(),
        refusing.join(", ")
    )))
}

fn state(resolution: &config::Resolution) -> &'static str {
    match resolution {
        config::Resolution::Absent { .. } => "absent",
        config::Resolution::Loaded(_) => "loaded",
    }
}

fn to_json(
    resolution: &config::Resolution,
    table: &config::mount::MountTable,
    bridged: &[config::bridge::Bridged],
) -> Value {
    json!({
        "path": resolution.path().display().to_string(),
        "state": state(resolution),
        "file_rev": resolution.file_rev(),
        "fingerprint": resolution.config().and_then(config::Config::fingerprint),
        "clear": table.is_clear(),
        "mounts": table.mounts().iter().map(|m| json!({
            "name": m.name(),
            "path": m.declared_path(),
            "canonical": m.canonical_path().map(|p| p.display().to_string()),
            "kind": m.kind().as_str(),
            "vault": m.vault(),
            "pin": m.pin(),
            "declared_name": m.declared_name(),
            // The reason word is the SAME spelling the human line carries. Two
            // spellings of one state is how a downstream reader and an operator
            // come to disagree about what the engine said.
            "state": m.state().word(),
            "detail": m.state().detail(),
        })).collect::<Vec<_>>(),
        "tools": resolution.tools().iter().map(|t| json!({
            "name": t.name,
            "kind": t.kind,
            "config": t.config,
        })).collect::<Vec<_>>(),
        // The bridge period (U9). `mount` is null on anything but agreement —
        // that is "the file wins" as data, not as prose.
        "bridge": bridged.iter().map(|b| json!({
            "var": b.var().name(),
            "state": b.state().word(),
            "mount": b.mount(),
            "canonical": b.canonical().map(|p| p.display().to_string()),
            "report": b.report(),
        })).collect::<Vec<_>>(),
    })
}

fn render_human(
    resolution: &config::Resolution,
    table: &config::mount::MountTable,
    bridged: &[config::bridge::Bridged],
) -> String {
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

    let _ = writeln!(out, "mounts ({}):", table.mounts().len());
    for m in table.mounts() {
        // The three legs of the map, then the state word. The canonical path is
        // printed only when it differs from the declared spelling, because that
        // difference IS the symlink the mount law collapses — printing it always
        // would bury the one line that matters.
        let _ = write!(
            out,
            "  {}  {}  {}",
            m.name(),
            m.kind().as_str(),
            m.declared_path()
        );
        if let Some(canonical) = m
            .canonical_path()
            .filter(|c| c.as_os_str() != m.declared_path())
        {
            let _ = write!(out, "  -> {}", canonical.display());
        }
        if let Some(vault) = m.vault() {
            let _ = write!(out, "  vault:{vault}");
        }
        if let Some(pin) = m.pin() {
            let _ = write!(out, "  pin:{pin}");
        }
        let _ = write!(out, "  {}", m.state().word());
        out.push('\n');
        if m.state().refuses() {
            let _ = writeln!(out, "      {}", m.state().detail());
        }
    }
    let _ = writeln!(out, "tools ({}):", resolution.tools().len());
    for t in resolution.tools() {
        let _ = writeln!(out, "  {}  {}", t.name, t.kind);
    }

    // The bridge period. Every variable is listed with its state word — an
    // operator must be able to see that a variable AGREES, not only that it
    // failed to complain. The report line appears only on the first divergence
    // in this process.
    let _ = writeln!(out, "bridge ({}):", bridged.len());
    for b in bridged {
        let _ = write!(out, "  {}  {}", b.var().name(), b.state().word());
        if let Some(mount) = b.mount() {
            let _ = write!(out, "  -> {mount}");
        }
        if let Some(canonical) = b.canonical() {
            let _ = write!(out, "  {}", canonical.display());
        }
        out.push('\n');
        if let Some(report) = b.report() {
            let _ = writeln!(out, "      {report}");
        }
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
