//! `mrd config` — the config plane's publishing surface.
//!
//! ```text
//! mrd config [--json]
//! ```
//!
//! Resolves the `MERIDIAN.md` bootstrap chain and prints what it found: the resolved path, the
//! state, the config's own rev and fingerprint, the bound mount table with each root's state,
//! and the declared tools in document order. Read-only.
//!
//! The mount table is the single authority for the three-way translation — canonical root name ↔
//! Obsidian vault name ↔ local path — so this verb prints all three legs per root plus the state
//! binding decided, and prints the canonical path beside the declared one whenever the two
//! differ: that difference is the symlink the mount law exists to collapse.
//!
//! `mrd read` cannot substitute: it routes through the render face, which elides every
//! `meridian-*` block (`ToonRenderer::with_meridian_elision`) and leaves no marker, so a reader
//! sees the prose, none of the blocks, and exit 0. Pinned by
//! `testsuite/tests/meridian_md.rs::the_render_face_elides_config_blocks_and_leaves_no_marker`.

use serde_json::{Value, json};

use crate::{Fail, Format};

/// Run `mrd config [--json]`. A positional argument or an unknown flag is already an exit-2
/// from the shared argument parser (`NO_PATH`). Errors [`Fail`] exit 1 when the chain refuses —
/// a malformed config, a stated path that is not a readable regular file, or an unbuildable
/// `$HOME`.
pub(crate) fn run(format: Format) -> Result<(), Fail> {
    let env = config::Env::from_process();
    // The origin is read from the same env the resolution is: the chain's answer to "which rung"
    // is what the resolved path cannot say for itself when both rungs name one file.
    let rung = config::rung(&env).map_err(|e| Fail::findings(e.to_string()))?;
    // The refusal rides verbatim — it already names what is broken, where, that nothing loaded,
    // and the fix.
    let resolution = config::resolve(&env).map_err(|e| Fail::findings(e.to_string()))?;
    // A bind refusal rides verbatim for the same reason a parse refusal does:
    // it already names the mount, the line, that nothing loaded, and the fix.
    let table = resolution
        .bind()
        .map_err(|e| Fail::findings(e.to_string()))?;

    // The bridge check never changes the exit code: a divergence between an env var and the file
    // is a note, because failing loud here would brick the CLI on every machine that exports it.
    let bridged = config::bridge::check(&config::bridge::BridgeEnv::from_process(), &table);

    match format {
        Format::Json => {
            println!(
                "{}",
                serde_json::to_string_pretty(&to_json(&resolution, &table, rung, &bridged))
                    .expect("json")
            );
        }
        Format::Human => print!("{}", render_human(&resolution, &table, rung, &bridged)),
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
    rung: config::Rung,
    bridged: &[config::bridge::Bridged],
) -> Value {
    json!({
        "path": resolution.path().display().to_string(),
        "state": state(resolution),
        // The same word the human line carries — one spelling across both faces.
        "origin": rung.word(),
        "file_rev": resolution.file_rev(),
        "fingerprint": resolution.config().and_then(config::Config::fingerprint),
        "clear": table.is_clear(),
        "mounts": table.mounts().iter().map(|m| json!({
            "name": m.name(),
            "path": m.declared_path(),
            "canonical": m.canonical_path().map(|p| p.display().to_string()),
            "kind": m.kind().as_str(),
            "primary": m.primary(),
            "vault": m.vault(),
            "pin": m.pin(),
            "declared_name": m.declared_name(),
            // The same spelling the human line carries.
            "state": m.state().word(),
            "detail": m.state().detail(),
        })).collect::<Vec<_>>(),
        "tools": resolution.tools().iter().map(|t| json!({
            "name": t.name,
            "kind": t.kind,
            "config": t.config,
        })).collect::<Vec<_>>(),
        // `mount` is null on anything but agreement — "the file wins" as data, not as prose.
        "bridge": bridged.iter().map(|b| json!({
            "var": b.var().name(),
            "state": b.state().word(),
            "mount": b.mount(),
            "canonical": b.canonical().map(|p| p.display().to_string()),
            "report": b.report(),
        })).collect::<Vec<_>>(),
    })
}

/// The human face's marker for a leg that is absent by construction. `(none)` is the tree's
/// existing spelling for an absent labelled scalar (`—` is the table-cell spelling).
///
/// Human face only: `--json` states the same fact as `null` at a present key, since a client
/// string-comparing `vault` would read a marker as a vault actually named that.
const ABSENT_LEG: &str = "(none)";

fn render_human(
    resolution: &config::Resolution,
    table: &config::mount::MountTable,
    rung: config::Rung,
    bridged: &[config::bridge::Bridged],
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();

    // The resolved path verbatim — the spelling a refusal from the same chain carries — then the
    // origin, because when a stale `MERIDIAN_CONFIG` names the default file the resolved path is
    // identical either way and only this word tells the two rungs apart.
    let _ = write!(
        out,
        "{}  {}  origin:{}",
        resolution.path().display(),
        state(resolution),
        rung.word()
    );
    if let Some(rev) = resolution.file_rev() {
        let _ = write!(out, "  file_rev:{rev}");
    }
    if let Some(fp) = resolution.config().and_then(config::Config::fingerprint) {
        let _ = write!(out, "  fp:{fp}");
    }
    out.push('\n');

    if matches!(resolution, config::Resolution::Absent { .. }) {
        // Absent is neither an error nor a warning: every machine starts here, and saying so
        // keeps an operator from reading a bare empty table as a failure.
        out.push_str("no config file — single-root behaviour, unchanged\n");
    }

    let _ = writeln!(out, "mounts ({}):", table.mounts().len());
    for m in table.mounts() {
        // The three legs of the map, then the state word. The canonical path prints only when it
        // differs from the declared spelling — that difference is the symlink the mount law
        // collapses.
        let _ = write!(
            out,
            "  {}  {}{}  {}",
            m.name(),
            m.kind().as_str(),
            // The same spelling the wire carries (`primary`), printed beside
            // the kind — the designation is a role, not a fourth map leg.
            if m.primary() { " primary" } else { "" },
            m.declared_path()
        );
        if let Some(canonical) = m
            .canonical_path()
            .filter(|c| c.as_os_str() != m.declared_path())
        {
            let _ = write!(out, "  -> {}", canonical.display());
        }
        // The vault leg always prints, with the marker when absent: `Mount::vault` is `Some` iff
        // `kind: vault` (the parser refuses a `vault:` line on a `git-folder` entry), so a
        // git-folder root's vault name cannot exist rather than merely being missing. Dropping
        // the cell would be byte-identical to a build that lost the name after the parser.
        let _ = write!(out, "  vault:{}", m.vault().unwrap_or(ABSENT_LEG));
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

    // Every variable is listed with its state word, so an operator can see that one agrees and
    // not merely that it failed to complain. The report line appears only on the first divergence
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
        // The render face elides these blocks and leaves no marker, so naming it here stops the
        // elision from reading as a parse failure.
        out.push_str(
            "note: meridian-* blocks are elided by the render face, so `mrd read` on this file\n\
             \x20     shows its prose and none of the entries above. This verb publishes them.\n",
        );
    }
    out
}
