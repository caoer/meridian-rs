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
/// the artifact, and the pinned pages from disk. The one call the serve path makes.
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

// ── the write edge: the attest path ───────────────────────────────────────────

/// The ARM act's disk session — the attest path the binding law's refusals
/// name as the legal road (`policy::binding` row 2: a direct door write to the
/// artifact is `BindingBreak`; "arm or disarm through the attest/arm path").
///
/// This edge does not ride the caller door, deliberately: the door has no
/// whole-page replace, `create` gate-refuses the artifact on a once-armed
/// workspace, and the act's own law is `policy::armed::arm`'s faults — the
/// attestation is validated BEFORE this session ever opens. What the session
/// owns is purely the disk protocol: the workspace write flock from read to
/// commit (no TOCTOU against another writer), rename-atomic byte landing, and
/// the crash order artifact-THEN-marker — a crash between the two leaves
/// artifact-without-marker, which reads as never-armed: the safe, re-runnable
/// state. The marker landing is the act's commit point. To every OTHER
/// process this is an external write, observed exactly as an editor's save —
/// the watch/resync plane already owns that class.
pub struct ArmSession {
    lock: fs::WriteLock,
}

/// What an [`ArmSession::commit`] landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmLanding {
    /// Whether THIS commit created the once-armed marker — the genesis arm
    /// (armed-plane rung 4). Permanent: the marker is never removed.
    pub first_arm: bool,
}

impl ArmSession {
    /// Open the attest path: take the workspace write flock. Refuses busy
    /// (`WouldBlock`) exactly as the door does — one lock, one writer.
    ///
    /// # Errors
    /// Any I/O failure at the lock file; `WouldBlock` when another meridian
    /// writer holds it (transient; retry).
    pub fn open(root: &fs::WorkspaceRoot) -> std::io::Result<Self> {
        Ok(Self {
            lock: fs::WriteLock::acquire(root)?,
        })
    }

    /// The artifact bytes as they stand UNDER the lock, or `None` when absent.
    /// The caller parses strictly and composes; a corrupt artifact must refuse
    /// the act before [`ArmSession::commit`] is ever reached.
    #[must_use]
    pub fn artifact(&self) -> Option<String> {
        read_artifact(self.lock.root())
    }

    /// Land the rendered artifact page and, on a never-armed workspace, the
    /// once-armed marker — artifact FIRST, then marker (the crash order the
    /// type's docs state). Consumes the session; the flock releases on return.
    ///
    /// # Errors
    /// Any I/O failure landing either half. A failure after the artifact
    /// rename and before the marker leaves the never-armed, re-runnable state.
    pub fn commit(self, page: &str) -> std::io::Result<ArmLanding> {
        let root = self.lock.root();
        let rel = std::path::Path::new(fs::domain::ARMED_RULES_PATH);
        let candidate = model::candidate_of_body(fs::domain::ARMED_RULES_PATH, page.to_string());
        if root.0.join(rel).try_exists()? {
            fs::replace_file(root, rel, &candidate)?;
        } else {
            fs::create_file(root, rel, &candidate)?;
        }

        if once_armed(root) {
            return Ok(ArmLanding { first_arm: false });
        }
        let marker = root.0.join(fs::domain::ATTESTED_MARKER_PATH);
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&marker)?;
        fs::honest_sync(&file)?;
        if let Some(parent) = marker.parent() {
            fs::honest_sync_path(parent)?;
        }
        Ok(ArmLanding { first_arm: true })
    }
}
