//! `mrd config` — the config plane's publishing surface, in two legs.
//!
//! ```text
//! mrd config [--json]            the plane: path, state, rev, mount table, tools
//! mrd config get [KEY] [--json]  the value: what the `^config` block's config() returned
//! ```
//!
//! They read one file and answer about different things. The bare verb answers about the MOUNT
//! PLANE — a table this machine's engine binds, whose every root's state is the engine's
//! business. `get` answers about the USER'S OWN DATA — an arbitrary value the engine declares no
//! schema for and reads no key of (`docs/meridian-md-schema.md` §6a). So `get` binds no roots:
//! an unbound root is the mount plane's refusal to make, and making it here would cost a machine
//! its config over a root it was not asking about.
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
//! It also names WHOSE resolution this is. Both processes read their OWN environment through
//! `config::Env::from_process()` — this CLI here, and a serving daemon on every mount-addressed
//! path, not only the `mounts` op:
//!
//! | daemon site | serves |
//! |---|---|
//! | `registry/src/mounts.rs::serve` | the `mounts` op |
//! | `wire-serve/src/mount_corpus.rs::load_mounts_where` | `walk` (`registry/src/walk_op.rs`), `sql` (`registry/src/sql_op.rs`) |
//! | `wire-serve/src/positions.rs::machine_mounts` | cross-root link/position translation on the read plane |
//! | `wire-serve/src/write.rs::machine_mount_table` | the pin door and the write plane |
//!
//! So the table published here can differ from the one the engine binds on ANY of them, not
//! just on discovery. A client `MERIDIAN_CONFIG` never reaches that daemon. Ruled (a)
//! 2026-08-23, card `serving-daemon-holds-mount-table-ignores-meridian-config`.
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
    let rung = config::rung(&env).map_err(refused)?;
    // The refusal rides verbatim — it already names what is broken, where, that nothing loaded,
    // and the fix.
    let resolution = config::resolve(&env).map_err(refused)?;
    // A bind refusal rides verbatim for the same reason a parse refusal does:
    // it already names the mount, the line, that nothing loaded, and the fix.
    let table = resolution.bind().map_err(refused)?;

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

/// A bootstrap-chain refusal, carrying the scope the refusal path would otherwise lose.
///
/// The chain's words ride verbatim first — they already name what is broken, where, that
/// nothing loaded, and the fix. What they cannot say is WHOSE chain refused. Until this
/// existed the refusal path was silent on that, so an operator reading it concluded the engine
/// was equally broken, when a serving daemon resolving its own environment may be binding a
/// table perfectly well (and the converse: this CLI going green proves nothing about that
/// daemon — the success line already says so).
///
/// On **stderr**, with the diagnostic: stdout stays empty on a refusal, which is what "a
/// refused config publishes NO mount table" means and what `config_e2e.rs` and `mount_e2e.rs`
/// pin.
fn refused(e: impl std::fmt::Display) -> Fail {
    Fail::findings(format!(
        "{e}\nanswered by: {ANSWERED_BY}, from its own environment — {DAEMON_SCOPE}; this \
         refusal is THIS process's chain and says nothing about the table a daemon binds"
    ))
}

fn state(resolution: &config::Resolution) -> &'static str {
    match resolution {
        config::Resolution::Absent { .. } => "absent",
        config::Resolution::Loaded(_) => "loaded",
    }
}

/// The block id that addresses the config block. One spelling, and it is an
/// ADDRESS: nothing scans `MERIDIAN.md` for a starlark fence, so a page may
/// carry as many other starlark blocks as its author likes.
const CONFIG_ANCHOR: &str = "config";

/// The entry the block owes: a zero-argument function whose return value IS the
/// config. Same word as the anchor, because the block and its entry name one
/// thing.
const CONFIG_ENTRY: &str = "config";

/// The block's shape, printed by every refusal that means "there is no block
/// here to read" — a refusal on this door is a teaching moment or it is nothing.
const BLOCK_SHAPE: &str =
    "```starlark\ndef config():\n    return {\"repos_root\": \"/path/to/repos\"}\n```\n^config";

/// Run `mrd config get [KEY] [--json]`: resolve the same bootstrap chain, read
/// the `^config` block out of the file it names, evaluate it, and print what
/// `config()` returned — the whole value, or the top-level member `key` names.
///
/// The value is arbitrary data (`docs/meridian-md-schema.md` §6a): this door
/// declares no schema and reads no key of it. It prints the VALUE and no
/// provenance, so `repos=$(mrd config get repos_root)` is the intended use;
/// `mrd config` is where the plane's path/rev/table live.
///
/// Roots are deliberately NOT bound here. Binding is the mount plane's business
/// and an unbound root refuses `mrd config`; the config block is readable either
/// way, and coupling them would make one broken path cost the other.
///
/// # Errors
/// [`Fail`] exit 1 when the chain refuses, when the file carries no `^config`
/// block (or two), when the block is not a `starlark` fence, when the source
/// will not parse, faults, or defines no `config()`, when it returns something
/// that is not data, or when `key` names a member the config does not have.
pub(crate) fn run_get(key: Option<&str>, format: Format) -> Result<(), Fail> {
    let env = config::Env::from_process();
    // The same chain `mrd config` walks, refusing in the same words: one file
    // per machine, and a file broken enough to refuse the mount table is not a
    // file this door will read a value out of either.
    let resolution = config::resolve(&env).map_err(refused)?;
    let path = resolution.path().to_path_buf();
    if matches!(resolution, config::Resolution::Absent { .. }) {
        return Err(Fail::findings(format!(
            "no config file at {} — there is no config to get. \
             Fix: create that file and give it a config block:\n{BLOCK_SHAPE}",
            path.display()
        )));
    }

    // Re-read rather than carry the bytes out of the parse: a `Config` holds the
    // mount plane's findings, never the page, and this door needs the page.
    let raw = std::fs::read_to_string(&path).map_err(|e| {
        Fail::findings(format!(
            "{}: {e}. Fix: make the file readable, then run this again.",
            path.display()
        ))
    })?;
    let nodes = syntax::parse(&raw);
    let doc = model::build(raw, nodes);

    let source = config_block(&doc, &path)?;
    let block_id = format!("{}#^{CONFIG_ANCHOR}", path.display());
    let value = effects::eval_value(
        &effects::Rule::new(block_id, source),
        CONFIG_ENTRY,
        effects::EvalLimits::default(),
    )
    .map_err(|e| Fail::findings(e.to_string()))?;

    let selected = match key {
        None => value,
        Some(key) => member(value, key, &path)?,
    };

    match format {
        // The value alone on both faces — the difference is only how a scalar
        // is spelled, never what is published.
        Format::Json => println!(
            "{}",
            serde_json::to_string_pretty(&selected).expect("serialize a value that came from json")
        ),
        Format::Human => println!("{}", human_value(&selected)),
    }
    Ok(())
}

/// The `^config` block's starlark source, or the refusal that names what stands
/// in its place.
fn config_block(doc: &model::Document, path: &std::path::Path) -> Result<String, Fail> {
    let r#ref = model::Ref::anchor(CONFIG_ANCHOR)
        .expect("`config` is inside the block-id charset [A-Za-z0-9-]");
    let target = model::resolve(doc, &r#ref).map_err(|e| match e {
        model::ResolveError::NotFound => Fail::findings(format!(
            "{}: no block carries `^{CONFIG_ANCHOR}` — this machine declares no config. \
             Fix: add one, with the id on its own line under the closing fence:\n{BLOCK_SHAPE}",
            path.display()
        )),
        // The mint plane never picks between duplicates, and neither does this
        // door: merging two blocks would need a precedence rule nobody ruled on.
        model::ResolveError::Ambiguous(candidates) => Fail::findings(format!(
            "{}: {} blocks carry `^{CONFIG_ANCHOR}` — the config is ambiguous and nothing was \
             read. Fix: keep one and delete or rename the rest.",
            path.display(),
            candidates.len()
        )),
    })?;
    let (span, _rev) = run::address::host_code_block(doc, &target.span).ok_or_else(|| {
        Fail::findings(format!(
            "{}: `^{CONFIG_ANCHOR}` does not key a fenced code block. \
             Fix: the id belongs on its own line directly under a fence's closing line:\n{BLOCK_SHAPE}",
            path.display()
        ))
    })?;
    let block = run::fence::classify(doc, &span).map_err(|e| {
        Fail::findings(format!(
            "{}#^{CONFIG_ANCHOR}: {e}. Fix: the config block is a `starlark` fence.",
            path.display()
        ))
    })?;
    if !matches!(block.lang, run::fence::TaskLanguage::Starlark) {
        return Err(Fail::findings(format!(
            "{}#^{CONFIG_ANCHOR}: the config block is a `{}` fence; it must be `starlark`. \
             Fix: change the info string to `starlark` and return the config from `config()`.",
            path.display(),
            block.lang.as_str()
        )));
    }
    Ok(block.source)
}

/// One top-level member of the returned config, or the refusal naming what the
/// config does carry. A KEY is a mapping's member — no path grammar, no
/// silent `null` for an absent key.
fn member(value: Value, key: &str, path: &std::path::Path) -> Result<Value, Fail> {
    let Value::Object(mut map) = value else {
        return Err(Fail::findings(format!(
            "{}#^{CONFIG_ANCHOR}: `{CONFIG_ENTRY}()` returned {}, not a mapping, so `{key}` \
             addresses nothing. Fix: run `mrd config get` with no KEY to see the whole value.",
            path.display(),
            type_word(&value)
        )));
    };
    map.remove(key).ok_or_else(|| {
        let keys: Vec<&str> = map.keys().map(String::as_str).collect();
        Fail::findings(format!(
            "{}#^{CONFIG_ANCHOR}: the config has no `{key}`. Fix: it declares {}.",
            path.display(),
            if keys.is_empty() {
                "no keys at all".to_owned()
            } else {
                format!("[{}]", keys.join(", "))
            }
        ))
    })
}

/// The human spelling of one value: a string rides BARE so a shell can capture
/// it (`repos=$(mrd config get repos_root)` is the whole point of the verb), and
/// every other shape rides as the JSON it already is. Quoting the string here
/// would put two quote characters into every path this verb is asked for.
fn human_value(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null | Value::Bool(_) | Value::Number(_) => value.to_string(),
        Value::Array(_) | Value::Object(_) => {
            serde_json::to_string_pretty(value).expect("serialize a value that came from json")
        }
    }
}

/// The type word a refusal names — the reader's own vocabulary, not serde's.
fn type_word(value: &Value) -> &'static str {
    match value {
        Value::Null => "None",
        Value::Bool(_) => "a bool",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "a mapping",
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
/// Both processes read their own environment through the *same* call —
/// `config::Env::from_process()` here at [`run`], and a serving daemon at the sites the module
/// doc tables. The resolved path cannot say which one answered, so this does. One spelling
/// across both faces: the human line quotes it, `--json` carries it at `answered_by`.
///
/// Note what is NOT frozen in the daemon: the file's *contents*. It re-derives per call on a
/// blake3 of the bytes (`docs/wire-contract.md` § A.5), so this line is about *which file*,
/// never about staleness.
const ANSWERED_BY: &str = "this process";

/// How wide the other answer is — the half a reader acts on.
///
/// Naming only the `mounts` op (as this line first shipped) teaches that DISCOVERY diverges and
/// nothing else, so a reader concludes a wire `walk` or `sql` resolves against the table
/// printed here. It does not: those paths call
/// `wire-serve::mount_corpus::load_mounts_where`, which reads the daemon's environment like
/// every other mount-addressed path (module doc § table). One spelling, three faces — the
/// success line, the refusal ([`refused`]), and `docs/status.md`.
const DAEMON_SCOPE: &str = "a serving daemon reads ITS OWN environment on every mount-addressed \
                            path (`mounts`, `walk`, `sql`, cross-root read and write)";

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

    // WHOSE answer this is, and HOW WIDE the divergence is. Both processes resolve the same
    // chain through the same call (`config::Env::from_process()`): this CLI at
    // `config_cmd.rs`, and a serving daemon at every mount-addressed site the module doc
    // tables — each from its OWN environment. So a table published here can differ from the
    // one the engine binds on any of them, and until this line existed nothing in either
    // output said which process answered. Printed unconditionally, because the divergence is
    // not limited to the override rung: a daemon started under a different `$HOME` resolves a
    // different rung-2 file with no variable set anywhere.
    let _ = writeln!(
        out,
        "answered by: {ANSWERED_BY}, from its own environment — {DAEMON_SCOPE}, so a wire \
         client can be served a different table"
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
