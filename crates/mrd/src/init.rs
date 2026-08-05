//! `mrd init` — declare the root (`MERIDIAN.md`, `type: meridian-root`),
//! register the drawer sentinel, and run M2 reconciliation (decision 0001
//! round 4, amendment M2; marker-retirement ruling 2026-07-26).
//!
//! # What init writes, and why it is this file
//! The retired marker file was existence-defined and untyped, so anything
//! carrying its name was believed. The artifact that now MEANS "this
//! directory is a meridian root" is the root's own self-declaration, read by
//! [`config::mount::read_root_declaration`]: the mount table binds a root by
//! the `name:` it declares and pins it, and `crates/run` reads `run.caps.*`
//! and `run.timeout_secs` out of it. So init writes THAT, and validity has one
//! owner — init writes the bytes, then reads them back through `config` and
//! reports the name `config` read.
//!
//! # What init does NOT do
//! It does not anchor the resolution ladder. The declaration plane is
//! `config`'s; the ladder answers `MERIDIAN_WORKSPACE` → nearest `.git` → the
//! cwd default and never reads a declaration (existence-only detection is
//! exactly what the marker got wrong). A tree declared BELOW a git root
//! therefore still resolves to the git root — so init reports the ladder's
//! answer for the target, tier and root, and names the fix when the two
//! differ. The ruling's "never silently" applies to this surface too.

use std::fs;
use std::path::{Path, PathBuf};

use config::mount::{DECLARATION_FILENAME, DECLARATION_TYPE, DeclarationFault};
use serde_json::json;

use crate::gc;
use crate::{Fail, Format, current_dir};

/// The declaration body init writes. No timestamp — a committed file must not churn. `type:` /
/// `version:` / `name:` are the owner's required keys ([`config::mount::DECLARATION_KEYS`]),
/// spelled through the owner's own constants so this writer cannot drift from the reader.
///
fn declaration_body(name: &str) -> String {
    format!(
        "---\ntype: {DECLARATION_TYPE}\nversion: {version}\nname: {name}\n---\n\n# {name}\n\nThis directory is a meridian root. `mrd init` wrote this declaration; the\nmount table binds this root by the `name:` above, and `[run.caps]`-style\nconventions are declared here as `run.caps.<pattern>:` frontmatter keys.\n",
        version = config::VERSION,
    )
}

/// What init found (or made) of the root's self-declaration.
enum Declared {
    /// Init wrote the declaration; `config` read this name back out of it.
    Written(String),
    /// A valid declaration was already there, left byte-for-byte.
    Already(String),
}

impl Declared {
    fn name(&self) -> &str {
        match self {
            Self::Written(n) | Self::Already(n) => n,
        }
    }

    fn state(&self) -> &'static str {
        match self {
            Self::Written(_) => "created",
            Self::Already(_) => "already declared",
        }
    }
}

/// Parse `init [PATH] [--name NAME] [--json]`.
fn parse(tail: &[String]) -> Result<(Option<String>, Option<String>, Format), Fail> {
    let mut path: Option<String> = None;
    let mut name: Option<String> = None;
    let mut json = false;
    let mut args = tail.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--name" => {
                let value = args
                    .next()
                    .ok_or_else(|| Fail::tool("--name needs a NAME".to_owned()))?;
                name = Some(value.clone());
            }
            flag if flag.starts_with('-') => {
                return Err(Fail::tool(format!("unknown flag: {flag}")));
            }
            _ if path.is_none() => path = Some(arg.clone()),
            value => return Err(Fail::tool(format!("unexpected argument: {value}"))),
        }
    }
    let format = if json { Format::Json } else { Format::Human };
    Ok((path, name, format))
}

/// Run `mrd init [PATH] [--name NAME] [--json]`.
///
/// # Errors
/// [`Fail`] exit 2 on a bad invocation, a refused deny ceiling, a present but
/// unreadable declaration, or an I/O failure.
pub(crate) fn dispatch(tail: &[String]) -> Result<(), Fail> {
    let (path, name, format) = parse(tail)?;
    run(path.as_deref(), name.as_deref(), format)
}

/// Run `mrd init`: deny-check, declaration, drawer sentinel, M2 reconcile.
pub(crate) fn run(
    target_arg: Option<&str>,
    name_arg: Option<&str>,
    format: Format,
) -> Result<(), Fail> {
    let cwd = current_dir()?;
    let target = resolve_target(&cwd, target_arg)?;

    // Deny ceiling BEFORE any write — refuse $HOME, /, mount points, the cache
    // root, etc. with a typed reason (exit 2). This is also what makes the
    // machine config unclobberable: `~/MERIDIAN.md` (`type: meridian-config`)
    // shares the reserved filename, and init can never reach $HOME to write it.
    if let Some(reason) = workspace::deny_reason(&target) {
        return Err(Fail::tool(format!(
            "refusing to init a workspace at {}: it is the {reason}",
            target.display()
        )));
    }

    let declaration_path = target.join(DECLARATION_FILENAME);
    let declared = declare(&target, &declaration_path, name_arg)?;

    // Register the drawer sentinel (records the canonical workspace path).
    let drawer = cache::CacheDrawer::open(&target);
    drawer.register().map_err(|e| {
        Fail::tool(format!(
            "cannot register the drawer for {}: {e}",
            target.display()
        ))
    })?;
    let persisted = !drawer.is_ephemeral();

    // M2 reconciliation + opportunistic auto-GC, only with a real cache root.
    let mut retired: Vec<String> = Vec::new();
    if let Ok(cache_root) = cache::cache_root() {
        retired = reconcile_descendants(&cache_root, &target)?;
        gc::maybe_auto_gc(&cache_root);
    }

    // What the LADDER says about this directory — the declaration does not
    // anchor it, so this is the sentence that keeps init honest.
    let answer = workspace::resolve(&target)
        .map_err(|e| Fail::tool(format!("cannot resolve {}: {e}", target.display())))?;

    report(
        format,
        &Report {
            target: &target,
            declaration_path: &declaration_path,
            declared: &declared,
            persisted,
            drawer: &drawer,
            retired: &retired,
            answer: &answer,
        },
    );
    Ok(())
}

/// Bring the root's self-declaration into being, or refuse.
///
/// Absent → write it, then read it back through the OWNER and report the name
/// `config` read. Already valid → leave it byte-for-byte. Present but not
/// readable as a declaration → refuse; init never overwrites another writer's
/// file (the same law `mrd skill hook`'s document states for a foreign hook).
fn declare(
    target: &Path,
    declaration_path: &Path,
    name_arg: Option<&str>,
) -> Result<Declared, Fail> {
    match config::mount::read_root_declaration(target) {
        Ok(existing) => Ok(Declared::Already(existing.name)),
        Err(DeclarationFault::Unreadable(reason)) => Err(Fail::tool(format!(
            "refusing to init {}: {} is present but does not read as a root declaration: {reason}",
            target.display(),
            declaration_path.display()
        ))),
        Err(DeclarationFault::Absent) => {
            let name = match name_arg {
                Some(n) => n.to_owned(),
                None => target
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            };
            fs::write(declaration_path, declaration_body(&name)).map_err(|e| {
                Fail::tool(format!(
                    "cannot write the root declaration {}: {e}",
                    declaration_path.display()
                ))
            })?;
            // Validity has ONE owner: ask it. A refusal here means the derived name is not a canonical
            // root name, so the file init just created is removed rather than left behind broken.
            //
            match config::mount::read_root_declaration(target) {
                Ok(written) => Ok(Declared::Written(written.name)),
                Err(fault) => {
                    let _ = fs::remove_file(declaration_path);
                    let reason = match fault {
                        DeclarationFault::Absent => "it vanished after the write".to_owned(),
                        DeclarationFault::Unreadable(reason) => reason,
                    };
                    Err(Fail::tool(format!(
                        "cannot declare {} as a root: {reason} Nothing was left on disk. Fix: rerun with `--name NAME`.",
                        target.display()
                    )))
                }
            }
        }
    }
}

/// Retire every drawer whose workspace is a strict DESCENDANT of `target` and which the ladder
/// no longer anchors on its own: with `target` registered, a daemon adopts such a tree from its
/// registered ancestor, so its own drawer is a leftover. Each retired sentinel records
/// `superseded_by = target` (amendment M2) so `cache clean` can reap it and a probe reads it as
/// retired. `target`'s own drawer is skipped.
///
///
///
///
///
///
///
///
///
///
fn reconcile_descendants(cache_root: &Path, target: &Path) -> Result<Vec<String>, Fail> {
    let drawers = cache::list_drawers(cache_root).map_err(|e| {
        Fail::tool(format!(
            "cannot list drawers under {}: {e}",
            cache_root.display()
        ))
    })?;
    let superseded_by = target.to_string_lossy().into_owned();
    let mut retired = Vec::new();
    for info in drawers {
        let ws = Path::new(&info.workspace);
        if ws == target || !ws.starts_with(target) || anchors_itself(ws) {
            continue;
        }
        cache::supersede(&info.drawer_dir, &superseded_by).map_err(|e| {
            Fail::tool(format!(
                "cannot retire descendant drawer {}: {e}",
                info.drawer_dir.display()
            ))
        })?;
        retired.push(info.workspace);
    }
    Ok(retired)
}

/// Whether the ladder anchors `ws` at `ws` itself, with no environment
/// override in play. An unreadable or vanished path anchors nothing.
fn anchors_itself(ws: &Path) -> bool {
    workspace::resolve_with_override(ws, None)
        .ok()
        .and_then(|answer| answer.root().map(|root| root == ws))
        .unwrap_or(false)
}

/// Canonicalize the target: the `PATH` argument (relative to cwd) or the cwd.
fn resolve_target(cwd: &Path, arg: Option<&str>) -> Result<PathBuf, Fail> {
    let raw = match arg {
        Some(p) if Path::new(p).is_absolute() => PathBuf::from(p),
        Some(p) => cwd.join(p),
        None => cwd.to_path_buf(),
    };
    workspace::canonicalize(&raw).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace path {}: {e}",
            raw.display()
        ))
    })
}

/// Everything the report renders, so the render stays one argument wide.
struct Report<'a> {
    target: &'a Path,
    declaration_path: &'a Path,
    declared: &'a Declared,
    persisted: bool,
    drawer: &'a cache::CacheDrawer,
    retired: &'a [String],
    answer: &'a workspace::Answer,
}

fn report(format: Format, r: &Report<'_>) {
    let drawer_dir = r.drawer.dir().map(|d| d.display().to_string());
    // The ladder's own provenance sentence (tier AND root) — `Answer`'s
    // `Display`, never reassembled here.
    let resolves = r.answer.to_string();
    // The declared root is the resolved one only when the ladder names it.
    let elsewhere = r.answer.root() != Some(r.target);
    match format {
        Format::Json => {
            let value = json!({
                "workspace": r.target.display().to_string(),
                "declaration": r.declaration_path.display().to_string(),
                "declared_name": r.declared.name(),
                "declaration_state": r.declared.state(),
                "drawer": drawer_dir,
                "drawer_persisted": r.persisted,
                "retired": r.retired,
                "resolves": resolves,
                "resolved_tier": r.answer.tier().word(),
                "resolved_root": r.answer.root_or_cwd().display().to_string(),
                "declared_root_is_resolved": !elsewhere,
            });
            println!("{}", serde_json::to_string_pretty(&value).expect("json"));
        }
        Format::Human => {
            println!("initialized workspace {}", r.target.display());
            println!(
                "  declared: {} as `{}` ({})",
                r.declaration_path.display(),
                r.declared.name(),
                r.declared.state()
            );
            match &drawer_dir {
                Some(d) if r.persisted => println!("  drawer:  {d} (registered)"),
                _ => println!("  drawer:  ephemeral (no cache root)"),
            }
            if r.retired.is_empty() {
                println!("  reconcile: no descendant drawers to retire");
            } else {
                println!(
                    "  reconcile: retired {} descendant drawer(s):",
                    r.retired.len()
                );
                for w in r.retired {
                    println!("    - {w}");
                }
            }
            println!("  resolves: {resolves}");
            if elsewhere {
                println!(
                    "  note: a declaration does not anchor the ladder, which resolves this path to {}. To make {} the resolved root, set MERIDIAN_WORKSPACE={}, or address it by name (`{}:`) through the mount table in $HOME/{DECLARATION_FILENAME}.",
                    r.answer.root_or_cwd().display(),
                    r.target.display(),
                    r.target.display(),
                    r.declared.name(),
                );
            }
        }
    }
}
