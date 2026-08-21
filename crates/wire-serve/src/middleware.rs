//! The middleware door's host half (armed-plane Part A2, wire-contract
//! § A.2.1): the overlay world `ctx.sql` / `ctx.read` query, and the
//! process-installed SQL backend seam.
//!
//! The backend seam exists because of the C2 topology law: `view` (the `DuckDB`
//! projection) is a write-only leaf no correctness crate may depend on, so
//! `wire-serve` cannot build the projection itself. A host that links `view`
//! (`mrd`, the resident daemon) installs a backend once at startup; a door
//! with no backend fails CLOSED when a middleware calls `ctx.sql` — never a
//! silent pass.

use std::collections::BTreeMap;
use std::sync::OnceLock;

/// One SQL projection call: the overlay world's parsed documents and one
/// SELECT, answered as [`policy::SqlRow`]s. Pure per call — the backend holds
/// no state between calls.
pub type SqlBackend = fn(&model::Docs, &str) -> Result<Vec<policy::SqlRow>, String>;

static SQL_BACKEND: OnceLock<SqlBackend> = OnceLock::new();

/// Install the process-wide `ctx.sql` backend. First install wins; a second
/// identical install is a no-op and a second different one is ignored (the
/// process has ONE projection engine). Hosts call this once at startup.
pub fn install_sql_backend(backend: SqlBackend) {
    let _ = SQL_BACKEND.set(backend);
}

/// The overlay world one middleware evaluation reads: the workspace snapshot
/// on disk, shadowed by the pending after-state of every file this sealed set
/// already carries (the caller's file, earlier middleware members, births).
pub(crate) struct DoorWorld<'a> {
    pub root: &'a fs::WorkspaceRoot,
    /// `workspace-relative path → pending bytes`, shadowing disk.
    pub overlay: &'a BTreeMap<String, String>,
}

/// A workspace-relative path is readable when it cannot escape the root —
/// read-only twin of the write door's confinement.
fn confined(rel: &str) -> bool {
    let p = std::path::Path::new(rel);
    !p.is_absolute()
        && p.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

impl policy::MwWorld for DoorWorld<'_> {
    fn read(&self, path: &str) -> Option<String> {
        if let Some(pending) = self.overlay.get(path) {
            return Some(pending.clone());
        }
        if !confined(path) {
            return None;
        }
        std::fs::read_to_string(self.root.0.join(path)).ok()
    }

    fn sql(&self, query: &str) -> Result<Vec<policy::SqlRow>, String> {
        let Some(backend) = SQL_BACKEND.get() else {
            return Err(
                "no ctx.sql backend is installed at this door — middleware SQL needs a \
                 view-backed host (mrd or the resident daemon); this process installed none"
                    .to_string(),
            );
        };
        backend(&overlay_docs(self.root, self.overlay)?, query)
    }
}

/// The overlay world as parsed documents: every hash-domain file on disk,
/// shadowed by the overlay, plus overlay-only paths (births). Unreadable or
/// non-UTF-8 disk members are skipped — the projection serves what parses,
/// exactly as the resident projection does.
fn overlay_docs(
    root: &fs::WorkspaceRoot,
    overlay: &BTreeMap<String, String>,
) -> Result<model::Docs, String> {
    let domain = fs::domain::Domain::load(root).map_err(|e| format!("domain load: {e}"))?;
    let rels = fs::hash_domain(root, &domain).map_err(|e| format!("domain walk: {e}"))?;
    let mut docs = BTreeMap::new();
    for rel in rels {
        let Some(rel_str) = rel.to_str() else {
            continue;
        };
        if overlay.contains_key(rel_str) {
            continue; // the pending tense shadows disk
        }
        let Ok(bytes) = std::fs::read_to_string(root.0.join(&rel)) else {
            continue;
        };
        docs.insert(
            rel_str.to_owned(),
            std::sync::Arc::new(doc_of(rel_str, &bytes)),
        );
    }
    for (rel, bytes) in overlay {
        docs.insert(rel.clone(), std::sync::Arc::new(doc_of(rel, bytes)));
    }
    Ok(docs)
}

/// Parse one member into a path-stamped document (the projection's grain).
fn doc_of(path: &str, bytes: &str) -> model::Document {
    let mut doc = model::build(bytes.to_string(), syntax::parse(bytes));
    if let model::NodeKind::Document { path: p, .. } = &mut doc.root.kind {
        *p = path.to_string();
    }
    doc
}
