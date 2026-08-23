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
//! It also names WHOSE resolution this is. Two processes answer mount-table questions through
//! the same `config::Env::from_process()` call — this CLI, and a serving daemon at
//! `crates/registry/src/mounts.rs` — each from its own environment, so the table published here
//! can differ from the one the engine binds. A client `MERIDIAN_CONFIG` never reaches that
//! daemon. Ruled (a) 2026-08-23, card `serving-daemon-holds-mount-table-ignores-meridian-config`.
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
        // WHICH PROCESS resolved the chain above. `origin` names the rung; this names the
        // resolver. A wire client asking the daemon gets that daemon's table, not this one.
        "answered_by": ANSWERED_BY,
        "file_rev": resolution.file_rev(),
        "fingerprint": resolution.config().and_then(config::Config::fingerprint),
        "clear": table.is_clear(),
        "mounts": table.mounts().iter().map(|m| json!({
            "name": m.name(),
            "path": m.declared_path(),
            "canonical": m.canonical_path().map(|p| p.display().to_string()),
            "primary": m.primary(),
            "vault": m.vault(),
            "pin": m.pin(),
            // The second lookup spelling (§5.1b); `null` when the block
            // declares none — a name is its own alias, so absence is the
            // majority row, not a missing fact.
            "alias": m.alias(),
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

/// Which process resolved the chain that produced this output.
///
/// Two processes answer mount-table questions through the *same* call —
/// `config::Env::from_process()` here at [`run`], and the daemon's `mounts` op at
/// `crates/registry/src/mounts.rs` — each reading its own environment. The resolved path cannot
/// say which one answered, so this does. One spelling across both faces: the human line quotes
/// it, `--json` carries it at `answered_by`.
///
/// Note what is NOT frozen in the daemon: the file's *contents*. It re-derives per call on a
/// blake3 of the bytes (`docs/wire-contract.md` § A.5), so this line is about *which file*,
/// never about staleness.
const ANSWERED_BY: &str = "this process";

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

    // WHOSE answer this is. Two processes resolve the same chain through the same call
    // (`config::Env::from_process()`): this CLI at `config_cmd.rs`, and a serving daemon at
    // `registry/src/mounts.rs` — each from its OWN environment. So a table published here can
    // differ from the one the engine binds, and until this line existed nothing in either
    // output said which process answered. Printed unconditionally, because the divergence is
    // not limited to the override rung: a daemon started under a different `$HOME` resolves a
    // different rung-2 file with no variable set anywhere.
    let _ = writeln!(
        out,
        "answered by: {ANSWERED_BY}, from its own environment — a serving daemon answers the \
         `mounts` op from ITS environment, so a wire client can be served a different table"
    );
    if matches!(rung, config::Rung::Override) {
        // Only on the override rung, because this is the case where an operator has
        // deliberately re-pointed the chain and is most likely to read the result as the
        // engine's. The daemon never sees the variable: its env was fixed at exec.
        let _ = writeln!(
            out,
            "      {} is read here and never reaches a serving daemon: the table below may \
             not be the one the engine binds",
            config::Rung::Override.word()
        );
    }

    let _ = writeln!(out, "mounts ({}):", table.mounts().len());
    for m in table.mounts() {
        // The three legs of the map, then the state word. The canonical path prints only when it
        // differs from the declared spelling — that difference is the symlink the mount law
        // collapses.
        let _ = write!(
            out,
            "  {}{}  {}",
            m.name(),
            // The same spelling the wire carries (`primary`) — the
            // designation is a role, not a fourth map leg.
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
        // the block declared `vault:` — presence IS vault-ness since the kind sweep
        // (ZT 2026-08-13). Dropping the cell would be byte-identical to a build that lost
        // the name after the parser.
        let _ = write!(out, "  vault:{}", m.vault().unwrap_or(ABSENT_LEG));
        if let Some(pin) = m.pin() {
            let _ = write!(out, "  pin:{pin}");
        }
        // The alias column prints only when declared: absence is the majority
        // row (a name is its own alias, §5.1b), so a marker on every line would
        // cost every reader to state nothing.
        if let Some(alias) = m.alias() {
            let _ = write!(out, "  alias:{alias}");
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
