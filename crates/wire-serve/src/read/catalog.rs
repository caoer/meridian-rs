//! One named document's generation-bound access at the serving seam.

use model::catalog::DocumentCatalog;
use std::collections::BTreeMap;
use wire::{ErrorBody, Path};

/// Compute over one named document while its body remains pinned. Known
/// catalog members must never fall through to a newer workspace file.
///
/// # Errors
/// Required member faults are `io_error`; unserved members retain their
/// per-file refusal. True nonmembers use the ordinary named live-source door.
pub fn with_document<T>(
    root: &fs::WorkspaceRoot,
    _catalog: &dyn DocumentCatalog,
    _unserved: &BTreeMap<String, String>,
    path: &Path,
    read: impl FnOnce(&model::Document) -> Result<T, Box<ErrorBody>>,
) -> Result<T, Box<ErrorBody>> {
    // Red compatibility prototype: the ordinary path loader cannot serve a
    // retained generation. Tests must distinguish this from catalog access.
    let doc = crate::load_doc(root, path)?;
    read(&doc)
}

#[cfg(test)]
mod tests;
