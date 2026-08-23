//! The per-invocation resolution flow (decision 0001 rounds 4-5, spec §1). The ladder's
//! answered rungs are pure filesystem functions ([`workspace::resolve`]); a root opens the
//! hashed drawer directly.

use std::fmt;
use std::path::{Path, PathBuf};

use cache::CacheDrawer;
use registry::Client;
use serde_json::json;
use workspace::{Base, ResolveError, Tier};

use crate::{Fail, Format, current_dir};

/// How the workspace for this invocation was resolved.
pub(crate) enum Source {
    /// The ladder answered — env override or git root — resolved purely, the
    /// hashed drawer opened directly.
    Direct(Tier),
    /// The ladder answered nothing, and a running daemon adopted the cwd from a
    /// registered ancestor.
    DaemonAdopted,
    /// The ladder answered nothing, but an ancestor holds a valid `MERIDIAN.md`
    /// root declaration (`type: meridian-root` — `mrd init`'s artifact): that
    /// declared root is the workspace, drawer opened directly.
    Declared,
    /// The ladder answered nothing and no daemon did either (or a registry
    /// miss): ephemeral, per-invocation, writes nothing. Only the lenient lane
    /// ([`resolve_runtime_lenient`]) surfaces this; the strict lane refuses
    /// instead ([`RuntimeResolveError::OutsideWorkspace`]).
    Ephemeral,
}

impl Source {
    /// A stable lowercase label for JSON / human output. Reuses [`Tier::word`] so the CLI
    /// cannot drift from the tier vocabulary it reports.
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Source::Direct(tier) => tier.word(),
            Source::DaemonAdopted => "daemon-adopted",
            Source::Declared => "declared",
            Source::Ephemeral => "ephemeral",
        }
    }
}

/// Why the strict resolution lane could not name a workspace.
pub(crate) enum RuntimeResolveError {
    /// The ladder itself failed (bad env override, unresolvable cwd).
    Ladder(ResolveError),
    /// PATH is outside every defined root: no env override, no `.git`
    /// ancestor, no ancestor `MERIDIAN.md` root declaration, no registered
    /// daemon root. The help's exit-2 leg ("PATH outside workspace") — the
    /// caller gets the refusal in milliseconds instead of a corpus walk of a
    /// tree that was never a workspace.
    OutsideWorkspace {
        /// The canonical path that failed to resolve.
        path: PathBuf,
    },
    /// A daemon answered with a registered ancestor, but that workspace no
    /// longer passes the defined-root test (no `.git` entry, no valid
    /// `MERIDIAN.md` root declaration). A leftover registry row is a hint,
    /// not a root — trusting it re-opened the corpus walk the refusal law
    /// closed (advisor gate 2026-08-20: a pre-fix walk's row served a
    /// 75-repo parent for 22.93 s while a fresh unmarked dir refused in
    /// 0.01 s). Same exit-2 leg, same milliseconds.
    StaleDaemonRoot {
        /// The path being resolved.
        path: PathBuf,
        /// The registered-but-unmarked workspace the daemon offered.
        workspace: PathBuf,
    },
}

impl fmt::Display for RuntimeResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ladder(e) => e.fmt(f),
            Self::OutsideWorkspace { path } => write!(
                f,
                "{} is outside a declared meridian workspace — no MERIDIAN_WORKSPACE override, \
                 no .git ancestor, no ancestor MERIDIAN.md root declaration, and no registered \
                 daemon root. Declare a root with `mrd init`, or address a bound root by name \
                 (`root:path`).",
                path.display()
            ),
            Self::StaleDaemonRoot { path, workspace } => write!(
                f,
                "{} is outside a declared meridian workspace — a daemon holds a leftover \
                 registration for {ws}, but that tree carries no workspace marker (no .git \
                 entry, no MERIDIAN.md root declaration), so it is not a defined root. Drop \
                 the stale row with `mrd unregister {ws}`, or declare the root with `mrd init`.",
                path.display(),
                ws = workspace.display()
            ),
        }
    }
}

/// The resolved workspace plus the drawer handle for this invocation.
pub(crate) struct Resolved {
    /// The canonical workspace path this invocation belongs to.
    pub(crate) workspace: PathBuf,
    /// How it was resolved.
    pub(crate) source: Source,
    /// The drawer handle — disk-backed for an answered rung (env override, git
    /// root) and for daemon-adopted trees, ephemeral for an unanchored tree
    /// with no daemon.
    pub(crate) drawer: CacheDrawer,
}

impl Resolved {
    /// Whether the drawer already holds a valid current-schema sentinel (warm)
    /// versus cold (miss). An ephemeral drawer is always cold.
    pub(crate) fn is_warm(&self) -> bool {
        matches!(self.drawer.probe(), cache::Probe::Hit(_))
    }
}

/// Resolve the workspace for `base` per the settled ladder — the STRICT lane
/// every corpus verb takes. A tree outside every defined root refuses
/// ([`RuntimeResolveError::OutsideWorkspace`], exit 2 at the callers) instead
/// of adopting the bare cwd and walking a corpus that was never a workspace
/// (measured 2026-08-20: `mrd rules` in an unmarked 75-repo parent adopted it
/// and walked 46k files for ~21 s to report nothing).
///
/// The [`Base`] is the door's answer to "did the operator name this path?" —
/// [`workspace::Base::Named`] for a PATH argument, [`workspace::Base::Cwd`] for
/// the process working directory. It decides whether `MERIDIAN_WORKSPACE` gets
/// to answer at all; see [`workspace::resolve_with_override`].
pub(crate) fn resolve_runtime(base: Base<'_>) -> Result<Resolved, RuntimeResolveError> {
    let resolved = resolve_runtime_lenient(base).map_err(RuntimeResolveError::Ladder)?;
    match resolved.source {
        Source::Ephemeral => Err(RuntimeResolveError::OutsideWorkspace {
            path: resolved.workspace,
        }),
        // A daemon adoption is trusted only while the registered workspace
        // still passes the defined-root test — a leftover row for an
        // unmarked tree refuses like ephemeral (exit 2 at the callers,
        // milliseconds). The check stays out of the lenient lane so
        // `mrd unregister` can still name and drop the stale registration.
        Source::DaemonAdopted if !config::mount::is_defined_root(&resolved.workspace) => {
            Err(RuntimeResolveError::StaleDaemonRoot {
                path: base.path().to_path_buf(),
                workspace: resolved.workspace,
            })
        }
        _ => Ok(resolved),
    }
}

/// The LENIENT lane: the same ladder, but an unanchored tree degrades to an
/// ephemeral resolution instead of refusing. Only for verbs whose job is
/// legitimate outside a defined root — `mrd unregister` (dropping a stale
/// drawer/registry entry must work after the markers are gone). Everything
/// else takes [`resolve_runtime`].
pub(crate) fn resolve_runtime_lenient(base: Base<'_>) -> Result<Resolved, ResolveError> {
    let answer = workspace::resolve(base)?;
    match answer.root() {
        // A rung answered: that root is the workspace, drawer opened directly.
        Some(root) => Ok(Resolved {
            drawer: CacheDrawer::open(root),
            source: Source::Direct(answer.tier()),
            workspace: root.to_path_buf(),
        }),
        // Unanchored: taking the cwd buys no registration — the daemon may adopt
        // it, otherwise the store is ephemeral.
        None => Ok(resolve_unanchored(
            base.path(),
            answer.root_or_cwd().to_path_buf(),
        )),
    }
}

/// An unanchored tree: ask the daemon, then look for an ancestor root
/// declaration, else degrade to ephemeral.
fn resolve_unanchored(cwd: &Path, bare_workspace: PathBuf) -> Resolved {
    // A socket that answers with a registered ancestor wins; any transport
    // failure (no daemon) or a miss falls through.
    if let Ok(client) = Client::from_default()
        && let Ok(Some(entry)) = client.resolve(cwd)
    {
        return Resolved {
            drawer: CacheDrawer::open(&entry.workspace),
            source: Source::DaemonAdopted,
            workspace: entry.workspace,
        };
    }
    // A declared-but-gitless root (`mrd init` without a repo): the nearest
    // ancestor whose MERIDIAN.md reads as a valid root declaration is the
    // workspace. Bounded like the `.git` rung; a stat per ancestor, one small
    // file parse on a hit — milliseconds, never a corpus walk.
    for dir in bare_workspace.ancestors().take(workspace::MAX_WALK_DEPTH) {
        if config::mount::read_root_declaration(dir).is_ok() {
            return Resolved {
                drawer: CacheDrawer::open(dir),
                source: Source::Declared,
                workspace: dir.to_path_buf(),
            };
        }
    }
    Resolved {
        drawer: CacheDrawer::ephemeral(&bare_workspace),
        source: Source::Ephemeral,
        workspace: bare_workspace,
    }
}

/// Run `mrd resolve [PATH]` — a read-only report of the resolution ladder for a
/// path (default cwd). It names the tier that answered and the root it named,
/// never a bare path (the ruling's "never silently"). Writes NOTHING: no drawer
/// creation, no registration, no auto-GC, so a cwd-default tree with no daemon
/// leaves the cache root untouched.
///
/// A `root:path` PATH takes the rooted lane instead ([`run_rooted`]): the
/// agent-plane answer — where the ref lands — while the ladder report stays
/// the ambient lanes' machine-bootstrap vocabulary (tier, drawer), the half
/// that never crossed to the MCP face.
pub(crate) fn run_command(target_arg: Option<&str>, format: Format) -> Result<(), Fail> {
    let cwd = current_dir()?;
    // The rooted lane (§4.1 colon law): a head-colon PATH is an agent-plane
    // address, never a literal path — canonicalizing the colon-bearing string
    // against the workspace was the leaked lookup this lane closes.
    if let Some(spelling) = target_arg.filter(|p| crate::rooted::is_rooted(p)) {
        return run_rooted(spelling, &cwd, format);
    }
    let base = match target_arg {
        Some(p) if Path::new(p).is_absolute() => PathBuf::from(p),
        Some(p) => cwd.join(p),
        None => cwd,
    };
    // `mrd resolve PATH` reports what the ladder says about the path the
    // operator NAMED; with no PATH it reports the ambient cwd, where the
    // override still answers. Reporting the override's tree under a named PATH
    // would answer a question nobody asked.
    let ladder_base = match target_arg {
        Some(_) => Base::Named(&base),
        None => Base::Cwd(&base),
    };
    let resolved = resolve_runtime(ladder_base).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            base.display()
        ))
    })?;
    let warm = resolved.is_warm();
    let drawer_dir = resolved.drawer.dir().map(|d| d.display().to_string());

    match format {
        Format::Json => {
            let value = json!({
                "workspace": resolved.workspace.display().to_string(),
                "source": resolved.source.label(),
                "ephemeral": resolved.drawer.is_ephemeral(),
                "drawer": drawer_dir,
                "state": if warm { "warm" } else { "cold" },
            });
            println!("{}", serde_json::to_string_pretty(&value).expect("json"));
        }
        Format::Human => {
            println!("workspace {}", resolved.workspace.display());
            println!("  source: {}", resolved.source.label());
            match &drawer_dir {
                Some(d) => println!("  drawer: {d} ({})", if warm { "warm" } else { "cold" }),
                None => println!("  drawer: ephemeral (cold, per-invocation)"),
            }
        }
    }
    Ok(())
}

/// The resolve door's §1 consequence clause — what did NOT happen because the
/// refusal fired ([`crate::path_law`] holds the family message; this door
/// states only its own name and consequence).
const RESOLVE_CONSEQUENCE: &str = "Nothing was resolved.";

/// Answer a rooted spelling: WHERE THE REF LANDS — the physical path, the
/// lane, the bound root and its workspace, and the canonical `root:path`
/// spelling. The MCP resolve face's answer, ported shape-for-shape; the
/// resolution is the read door's seam ([`crate::rooted`]), so two doors
/// cannot hold two opinions of one ref.
///
/// No existence check and no daemon contact: the mount table is the whole
/// authority, and the answer names where the ref lands whether or not a file
/// sits there today — absence is the engine's question (§5.6), never the
/// address plane's. A resolution refusal is the address answer (exit 1,
/// framed with the workspace the caller stands in), never a canonicalize of
/// the literal colon-bearing string.
fn run_rooted(spelling: &str, cwd: &Path, format: Format) -> Result<(), Fail> {
    use wire::{ErrorBody, ErrorCode};

    let refusal = |error: Box<ErrorBody>, format: Format| -> Result<(), Fail> {
        // The refusal frames with the workspace the caller stands in — no
        // target workspace exists to name. Lenient on purpose: the frame is a
        // label, and a rooted refusal must still print outside a workspace.
        let ambient = resolve_runtime_lenient(Base::Cwd(cwd))
            .map_err(|e| {
                Fail::tool(format!(
                    "cannot resolve workspace for {}: {e}",
                    cwd.display()
                ))
            })?
            .workspace;
        Err(crate::engine::json_refusal(format, &ambient, &error))
    };

    // Path grain, the MCP face's own `#` door: a fragment addresses a
    // section, not a file, and silently resolving the path half would answer
    // a question the caller did not ask.
    if spelling.contains('#') {
        let mut e = ErrorBody::new(ErrorCode::BadPath);
        e.path = Some(wire::Path(spelling.to_owned()));
        e.message = Some(format!(
            "{spelling} carries a `#` fragment, and resolve answers at path grain — a \
             fragment addresses a section, not a file. Re-issue with the bare `root:path` \
             (sections are the read door's: `mrd read PATH#FRAG` or `--section`). \
             {RESOLVE_CONSEQUENCE}"
        ));
        return refusal(Box::new(e), format);
    }
    match crate::rooted::resolve(spelling, "resolve", RESOLVE_CONSEQUENCE) {
        Ok((rel, rooted)) => {
            let landed = rooted.workspace.join(&rel);
            match format {
                // `root` and `ref` carry the MOUNT's name whichever spelling the
                // caller used — the canonical-spelling law (§ 4.6a). `alias` is
                // the spelling that landed, present only when it was not the
                // name, so a caller can see WHY `sessions:` reached this tree
                // without diffing two strings.
                Format::Json => {
                    let value = json!({
                        "workspace": rooted.workspace.display().to_string(),
                        "source": "rooted",
                        "root": rooted.name.as_str(),
                        "alias": rooted.alias.as_ref().map(addr::MountName::as_str),
                        "primary": rooted.primary,
                        "path": landed.display().to_string(),
                        "ref": rooted.canonical_ref(&rel),
                    });
                    println!("{}", serde_json::to_string_pretty(&value).expect("json"));
                }
                Format::Human => {
                    let via_alias = match &rooted.alias {
                        Some(alias) => format!(" (alias {alias})"),
                        None => String::new(),
                    };
                    println!("{}", landed.display());
                    println!("  lane: rooted — the ref names root {}", rooted.name);
                    println!(
                        "  root: {}{via_alias} (bound)  workspace:{}{}",
                        rooted.name,
                        rooted.workspace.display(),
                        if rooted.primary { "  ← primary" } else { "" }
                    );
                    println!("  ref: {}", rooted.canonical_ref(&rel));
                }
            }
            Ok(())
        }
        Err(error) => refusal(error, format),
    }
}
