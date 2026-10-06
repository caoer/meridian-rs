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
    catalog: &dyn DocumentCatalog,
    unserved: &BTreeMap<String, String>,
    path: &Path,
    read: impl FnOnce(&model::Document) -> Result<T, Box<ErrorBody>>,
) -> Result<T, Box<ErrorBody>> {
    if let Some(revision) = catalog.revision(&path.0) {
        let lease = catalog
            .pin_read(&path.0)
            .map_err(|error| retained_fault(path, error))?
            .ok_or_else(|| retained_fault(path, "known member has no body lease"))?;
        let doc = lease
            .document()
            .map_err(|error| retained_fault(path, error))?;
        if doc.root.node_rev != *revision {
            return Err(retained_fault(
                path,
                "body does not match the selected revision",
            ));
        }
        return read(doc);
    }
    if let Some(condition) = unserved.get(&path.0) {
        return Err(super::unserved_refusal(path, condition));
    }
    let doc = crate::load_doc(root, path)?;
    read(&doc)
}

pub(super) fn retained_fault(path: &Path, error: impl std::fmt::Display) -> Box<ErrorBody> {
    let mut refusal = ErrorBody::new(wire::ErrorCode::IoError);
    refusal.path = Some(path.clone());
    refusal.cause = Some(format!("cannot read retained member {}: {error}", path.0));
    Box::new(refusal)
}

#[cfg(test)]
mod tests;
