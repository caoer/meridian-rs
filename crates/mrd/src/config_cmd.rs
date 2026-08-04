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
//!   `meridian-*` block (`ToonRenderer::with_meridian_elision`). Measured on
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
//! # It reports the RESOLVED state, including where the resolution came from (U33)
//! The table printed is the **bound** one, never the file's literal blocks. The
//! same rule reaches the entry point itself: the line carries `origin:` — which
//! rung of the chain supplied the path.
//!
//! That word exists because of a measurement, not a preference. Before it,
//! `MERIDIAN_CONFIG=$HOME/MERIDIAN.md mrd config` and `mrd config` with the
//! variable unset printed **byte-identical output** — two environments differing
//! in exactly the variable the chain is made of, indistinguishable at the only
//! surface that publishes the chain. An operator debugging a stale exported
//! override could read the endpoint and never the path taken to it.
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
    let env = config::Env::from_process();
    // The ORIGIN is read from the same env the resolution is, and before it:
    // the chain's answer to "which rung" is what the resolved path cannot say
    // for itself when both rungs name one file (U33).
    let rung = config::rung(&env).map_err(|e| Fail::findings(e.to_string()))?;
    // The refusal rides verbatim: it already names what is broken, where, that
    // nothing loaded, and the fix. Re-wording it here would be a second
    // spelling of the same fact.
    let resolution = config::resolve(&env).map_err(|e| Fail::findings(e.to_string()))?;
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
        // The SAME word the human line carries, for the same reason the mount
        // state words are one spelling across both faces.
        "origin": rung.word(),
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

/// The human face's marker for a leg that is absent **by construction**.
///
/// **Reused, not minted (S3-R49).** The existing set was enumerated before a
/// spelling was proposed. **The unit counted is a Rust STRING LITERAL used as a
/// render filler — not a prose occurrence** (S3-R74), and the count is taken
/// **at `f21164c3`, this unit's base, over `crates/`**, because after this
/// commit the same command returns 3 more and they are all this unit's own:
///
/// ```text
/// git grep -n '"(none)"' f21164c3 -- 'crates/*.rs'    # 6
/// git grep -n '"—"'      f21164c3 -- 'crates/*.rs'    # 5
/// ```
///
/// - **`(none)` — 6**, every one filling a **labelled scalar**: `as_of:  (none)`
///   (`view_status.rs:174`), the `fingerprint_attempted` fallback
///   (`view_status.rs:183`), `as_of=`/`live=` in the freshness banner
///   (`sql.rs:963`, `:969`, `:977`), and an empty rev list (`test_cmd.rs:819`).
/// - **`—` — 5**, every one a **markdown table cell**: the scenario table
///   (`test_cmd.rs:767`, `:778`), the history table (`history_cmd.rs:573`), and
///   `fmt_opt`/`gate_cell` (`perfsuite/src/report.rs:198`, `:208`).
/// - **`-` — 1**, a walk row's rev (`walk_cmd.rs:345`).
///
/// **No collision:** none of them spells this leg today, because today this leg
/// has no spelling at all.
///
/// `(none)` is the one taken because it is the spelling this cell's SHAPE
/// already uses: `vault:{value}` is a labelled scalar in a whitespace row, which
/// is every `(none)` site and no `—` site. One state gets one spelling, and the
/// spelling that already exists wins.
///
/// It stays on this face only. `--json` states the same fact as `null` at a
/// present key, which is the machine's statement and needs no marker; a client
/// string-comparing `vault` would read this one as a vault actually named it.
const ABSENT_LEG: &str = "(none)";

fn render_human(
    resolution: &config::Resolution,
    table: &config::mount::MountTable,
    rung: config::Rung,
    bridged: &[config::bridge::Bridged],
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();

    // The resolved path VERBATIM, which is the spelling a refusal from the same
    // chain carries. Abbreviating it to `~/MERIDIAN.md` here would give one
    // file two spellings across two faces of one verb.
    //
    // Then the ORIGIN, because the path cannot say where it came from: when a
    // stale `MERIDIAN_CONFIG` happens to name the default file, the resolved
    // path is identical either way and only this word tells them apart (U33).
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
        // The vault leg is printed ALWAYS, with the marker when it is absent.
        //
        // This is the ONE structurally-partial axis of criterion 2's three-way
        // map: `Mount::vault` is `Some` iff `kind: vault`, because the parser
        // REFUSES a `vault:` line on a `git-folder` entry. A git-folder root's
        // vault name is therefore not missing — it cannot exist, and the
        // criterion asks this face to say which of the two it is looking at.
        //
        // Dropping the cell could not say that: `archive  git-folder  /…  bound`
        // is byte-identical to what a build that lost the vault name between the
        // parser and this line would print. **A blank cell is a dropped value**,
        // and the reader is the one asked to guess — U6's byte-identity lesson,
        // at the row level.
        //
        // The other two conditional cells are NOT this class and stay
        // conditional. `-> canonical` is suppressed when it EQUALS the declared
        // path (an equality, not an absence), and when it is genuinely `None`
        // the row already carries `grey(path-unseeable)` with its teaching
        // detail — a marker there would be a second spelling of one fact.
        // `pin:` is legal on BOTH kinds, so its absence is an operator's choice
        // rather than a property of the root, and it is not a leg of the map.
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
