//! The per-invocation resolution flow (decision 0001 rounds 4-5, spec §1).
//!
//! Tiers 1-3 are pure filesystem functions ([`workspace::resolve`]); a hit
//! opens the hashed drawer directly. A tier-4 bare tree asks a running daemon
//! whether the cwd sits inside a registered ancestor — a socket answer adopts
//! it, silence or a miss degrades to an ephemeral, per-invocation store that
//! writes NOTHING and never fails (shipped `NewStore("")` precedent). A bare
//! tree is never silently registered here.

use std::path::{Path, PathBuf};

use cache::CacheDrawer;
use registry::Client;
use serde_json::json;
use workspace::{ResolveError, Tier};

use crate::{Fail, Format, current_dir};

/// How the workspace for this invocation was resolved.
pub(crate) enum Source {
    /// Tiers 1-3: env override, `.meridian.toml` marker, or git root — resolved
    /// purely, the hashed drawer opened directly.
    Direct(Tier),
    /// A tier-4 bare tree adopted from a running daemon's registered ancestor.
    DaemonAdopted,
    /// A tier-4 bare tree with no daemon (or a registry miss): ephemeral,
    /// per-invocation, writes nothing.
    Ephemeral,
}

impl Source {
    /// A stable lowercase label for JSON / human output.
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Source::Direct(Tier::EnvOverride) => "env-override",
            Source::Direct(Tier::Marker) => "marker",
            Source::Direct(Tier::GitRoot) => "git-root",
            Source::Direct(Tier::Bare) => "bare",
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
    /// The drawer handle — disk-backed for tiers 1-3 and daemon-adopted trees,
    /// ephemeral for a bare tree with no daemon.
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
    let resolution = workspace::resolve(cwd)?;
    match resolution.tier {
        Tier::EnvOverride | Tier::Marker | Tier::GitRoot => Ok(Resolved {
            drawer: CacheDrawer::open(&resolution.workspace),
            source: Source::Direct(resolution.tier),
            workspace: resolution.workspace,
        }),
        Tier::Bare => Ok(resolve_bare(cwd, resolution.workspace)),
    }
}

/// A tier-4 bare tree: ask the daemon, else degrade to ephemeral.
fn resolve_bare(cwd: &Path, bare_workspace: PathBuf) -> Resolved {
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
/// path (default cwd). Writes NOTHING: no drawer creation, no registration, no
/// auto-GC, so a tier-4 bare tree with no daemon leaves the cache root untouched.
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
