//! The disk edge of a workspace's armed law — the reads both armed-law hosts
//! share.
//!
//! `policy` is I/O-free by charter, so the marker probe, the artifact read, and
//! the pinned-page source live here. The write door ([`crate::gate`]) and the
//! reaction feeder ([`crate::reaction`]) pivot on the same once-armed marker.

use std::cell::RefCell;
use std::collections::BTreeMap;

use policy::armed::PageSource;

/// Whether this workspace has ever been armed — read from the marker's
/// presence, never the artifact's: deleting the artifact must not read as
/// "never armed" (silent disarm). An ambiguous stat fails closed (assume
/// armed).
#[must_use]
pub fn once_armed(root: &fs::WorkspaceRoot) -> bool {
    root.0
        .join(fs::domain::ATTESTED_MARKER_PATH)
        .try_exists()
        .unwrap_or(true)
}

/// Read the attested armed-set artifact, or `None` when it is absent.
///
/// An artifact that exists but cannot be read is not `None` — that would be a
/// silent disarm. It reads as an empty page, which the resolver refuses as
/// corrupt.
#[must_use]
pub fn read_artifact(root: &fs::WorkspaceRoot) -> Option<String> {
    match std::fs::read_to_string(root.0.join(fs::domain::ARMED_RULES_PATH)) {
        Ok(page) => Some(page),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(String::new()),
    }
}

/// The pinned pages on disk, read once each.
///
/// The cache is for correctness, not speed: verification and loading must see
/// the same bytes, or a page edited between the two calls would load a
/// declaration the rev check never approved.
pub struct DiskPages<'a> {
    root: &'a fs::WorkspaceRoot,
    seen: RefCell<BTreeMap<String, String>>,
}

impl<'a> DiskPages<'a> {
    /// A page source rooted at `root`.
    #[must_use]
    pub fn new(root: &'a fs::WorkspaceRoot) -> Self {
        Self {
            root,
            seen: RefCell::new(BTreeMap::new()),
        }
    }
}

impl PageSource for DiskPages<'_> {
    /// Page paths reach here only from a parsed artifact row, which validated them
    /// as relative, non-escaping workspace paths at intake.
    fn read(&self, page: &str) -> std::io::Result<String> {
        if let Some(held) = self.seen.borrow().get(page) {
            return Ok(held.clone());
        }
        let bytes = std::fs::read_to_string(self.root.0.join(page))?;
        self.seen
            .borrow_mut()
            .insert(page.to_string(), bytes.clone());
        Ok(bytes)
    }
}

/// Resolve the armed law governing a write at `at_path`, reading the marker,
/// the artifact, and the pinned pages from disk. The one call both hosts make.
#[must_use]
pub fn resolve_at(root: &fs::WorkspaceRoot, at_path: &str) -> policy::ArmedLaw {
    let pages = DiskPages::new(root);
    policy::resolve_armed_law(
        read_artifact(root).as_deref(),
        once_armed(root),
        at_path,
        &pages,
        policy::CheckLimits::default(),
    )
}
