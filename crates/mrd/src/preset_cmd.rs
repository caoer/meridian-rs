//! Shared plumbing for the three preset verbs (`mrd new`, `mrd unfold`, `mrd
//! reconcile`): resolve the ambient workspace root, and resolve a
//! `<kind>`/`<preset>` token to its def page path.
//!
//! [`resolve_root`] is the wider of the two — every verb that needs the ambient
//! workspace dials it, `mrd realise` and `mrd journal` included, so the
//! resolution `pin`/`attest` run is the resolution they all run.

use crate::Fail;

/// Resolve the ambient workspace to a canonical [`fs::WorkspaceRoot`] (the same
/// resolution `pin`/`attest` run).
///
/// # Errors
/// A tool failure (exit 2) when the workspace cannot be resolved or canonicalized.
pub(crate) fn resolve_root() -> Result<fs::WorkspaceRoot, Fail> {
    let cwd = crate::current_dir()?;
    let resolved = crate::resolve::resolve_runtime(&cwd).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace for {}: {e}",
            cwd.display()
        ))
    })?;
    let canonical = workspace::canonicalize(&resolved.workspace).map_err(|e| {
        Fail::tool(format!(
            "cannot resolve workspace {} ({e})",
            resolved.workspace.display()
        ))
    })?;
    Ok(fs::WorkspaceRoot(canonical))
}

/// Resolve a `<kind>`/`<preset>` token to its def page path: a token that already
/// names a page (bears a `/` or a `.md` suffix) is used verbatim; a bare kind
/// resolves to the conventional `presets/<kind>.md`.
pub(crate) fn def_path(token: &str) -> String {
    let names_page = token.contains('/')
        || std::path::Path::new(token)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
    if names_page {
        token.to_owned()
    } else {
        format!("presets/{token}.md")
    }
}
