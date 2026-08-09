//! The per-invocation resolution flow (decision 0001 rounds 4-5, spec §1). The ladder's
//! answered rungs are pure filesystem functions ([`workspace::resolve`]); a root opens the
//! hashed drawer directly.

use std::path::{Path, PathBuf};

use cache::CacheDrawer;
use registry::Client;
use serde_json::json;
use workspace::{ResolveError, Tier};

use crate::{Fail, Format, current_dir};

/// How the workspace for this invocation was resolved.
pub(crate) enum Source {
    /// The ladder answered — env override or git root — resolved purely, the
    /// hashed drawer opened directly.
    Direct(Tier),
    /// The ladder answered nothing, and a running daemon adopted the cwd from a
    /// registered ancestor.
    DaemonAdopted,
    /// The ladder answered nothing and no daemon did either (or a registry
    /// miss): ephemeral, per-invocation, writes nothing.
    Ephemeral,
}

impl Source {
    /// A stable lowercase label for JSON / human output. Reuses [`Tier::word`] so the CLI
    /// cannot drift from the tier vocabulary it reports.
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Source::Direct(tier) => tier.word(),
            Source::DaemonAdopted => "daemon-adopted",
            Source::Ephemeral => "ephemeral",
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

/// Resolve the workspace for `cwd` per the settled ladder.
pub(crate) fn resolve_runtime(cwd: &Path) -> Result<Resolved, ResolveError> {
    let answer = workspace::resolve(cwd)?;
    match answer.root() {
        // A rung answered: that root is the workspace, drawer opened directly.
        Some(root) => Ok(Resolved {
            drawer: CacheDrawer::open(root),
            source: Source::Direct(answer.tier()),
            workspace: root.to_path_buf(),
        }),
        // Unanchored: taking the cwd buys no registration — the daemon may adopt
        // it, otherwise the store is ephemeral.
        None => Ok(resolve_unanchored(cwd, answer.root_or_cwd().to_path_buf())),
    }
}

/// An unanchored tree: ask the daemon, else degrade to ephemeral.
fn resolve_unanchored(cwd: &Path, bare_workspace: PathBuf) -> Resolved {
    // A socket that answers with a registered ancestor wins; any transport
    // failure (no daemon) or a miss falls through to ephemeral.
    if let Ok(client) = Client::from_default()
        && let Ok(Some(entry)) = client.resolve(cwd)
    {
        return Resolved {
            drawer: CacheDrawer::open(&entry.workspace),
            source: Source::DaemonAdopted,
            workspace: entry.workspace,
        };
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
pub(crate) fn run_command(target_arg: Option<&str>, format: Format) -> Result<(), Fail> {
    let cwd = current_dir()?;
    let base = match target_arg {
        Some(p) if Path::new(p).is_absolute() => PathBuf::from(p),
        Some(p) => cwd.join(p),
        None => cwd,
    };
    let resolved = resolve_runtime(&base).map_err(|e| {
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
