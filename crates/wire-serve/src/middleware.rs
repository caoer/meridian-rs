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
//!
//! One door run holds one [`DoorWorld`] across all its middleware rows, so a
//! `ctx.sql` pays for a projection only when the world it reads has moved
//! (armed-plane § The ctx surface).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// One built `ctx.sql` projection of one overlay world. It answers any number
/// of SELECTs. A call's DML, DDL or `SET` must never reach a later call: the
/// backend undoes what it can and reports the rest as [`SqlProjection::spent`].
pub trait SqlProjection: Send {
    /// Run one SELECT against the projection and answer its rows.
    ///
    /// # Errors
    /// The backend's own message — a SQL error or a connection failure.
    /// [`DoorWorld`] drops a projection whose call errored.
    fn query(&mut self, query: &str) -> Result<Vec<policy::SqlRow>, String>;

    /// Whether an answered call left the projection changed in a way the
    /// backend could not undo. [`DoorWorld`] then drops it, and the next call
    /// builds a fresh one.
    fn spent(&self) -> bool;
}

/// Build the projection of one overlay world from its parsed documents.
pub type SqlBackend = fn(&model::Docs) -> Result<Box<dyn SqlProjection>, String>;

static SQL_BACKEND: OnceLock<SqlBackend> = OnceLock::new();

/// Install the process-wide `ctx.sql` backend. First install wins; a second
/// identical install is a no-op and a second different one is ignored (the
/// process has ONE projection engine). Hosts call this once at startup.
pub fn install_sql_backend(backend: SqlBackend) {
    let _ = SQL_BACKEND.set(backend);
}

/// The overlay world one door run's middleware reads: the workspace snapshot
/// on disk, shadowed by the pending bytes of every file the sealed set
/// already carries (the caller's file, earlier middleware members, births).
///
/// The door stages pending bytes between rows ([`DoorWorld::stage`]) and hands
/// `&self` to each evaluation. What `ctx.sql` builds is kept here:
///
/// - the parsed disk members, built on the first `ctx.sql` and kept for the
///   whole run, so the run reads one disk snapshot;
/// - the projection over disk + pending as they stand now, dropped by any
///   [`DoorWorld::stage`] that changes pending bytes.
///
/// Staging takes `&mut self`, so no evaluation can hold the world while it
/// moves: a kept projection is always the projection of the current world.
///
/// Public because the RUN plane mounts the same door over its own pending
/// splice (`run::gate`, U4.2 byte-landing parity): the run plane lands bytes
/// through `fs::apply_batch` rather than the wire choke-point, so it mounts
/// this world rather than re-spelling what "the overlay world" is — and it
/// inherits the same installed `ctx.sql` backend, which is the whole point of
/// there being ONE of these.
pub struct DoorWorld<'a> {
    /// The workspace the un-shadowed reads resolve under.
    root: &'a fs::WorkspaceRoot,
    /// `workspace-relative path → pending bytes`, shadowing disk.
    pending: BTreeMap<String, String>,
    /// What `ctx.sql` has built over this world so far.
    built: Mutex<Built>,
}

/// The `ctx.sql` state one [`DoorWorld`] keeps.
#[derive(Default)]
struct Built {
    /// Every readable hash-domain member on disk, parsed — pending paths
    /// included, so staging never invalidates it.
    disk: Option<model::Docs>,
    /// The projection of disk + the current pending bytes.
    projection: Option<Box<dyn SqlProjection>>,
}

impl<'a> DoorWorld<'a> {
    /// A world with nothing pending yet: disk alone.
    #[must_use]
    pub fn new(root: &'a fs::WorkspaceRoot) -> Self {
        DoorWorld {
            root,
            pending: BTreeMap::new(),
            built: Mutex::new(Built::default()),
        }
    }

    /// Stage `bytes` as `path`'s pending after-state, shadowing disk and any
    /// earlier pending bytes for that path. Changed bytes drop the kept
    /// projection; identical bytes keep it.
    pub fn stage(&mut self, path: &str, bytes: &str) {
        if self.pending.get(path).is_some_and(|held| held == bytes) {
            return;
        }
        self.pending.insert(path.to_owned(), bytes.to_owned());
        self.built
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .projection = None;
    }

    /// The world's documents: the parsed disk members (parsed now if this is
    /// the run's first build), shadowed by the pending bytes, parsed.
    fn docs(&self, built: &mut Built) -> Result<model::Docs, String> {
        let building = timing::phase("mw.sql.world");
        if built.disk.is_none() {
            built.disk = Some(disk_docs(self.root)?);
        }
        let mut docs = built.disk.clone().unwrap_or_default();
        for (rel, bytes) in &self.pending {
            docs.insert(rel.clone(), Arc::new(doc_of(rel, bytes)));
        }
        building.stop();
        Ok(docs)
    }
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
        if let Some(pending) = self.pending.get(path) {
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
        let mut built = self.built.lock().unwrap_or_else(PoisonError::into_inner);
        let mut projection = match built.projection.take() {
            Some(kept) => kept,
            None => backend(&self.docs(&mut built)?)?,
        };
        let rows = projection.query(query);
        // A call that errored or spent the projection may have changed it;
        // the next call builds from the documents again.
        if rows.is_ok() && !projection.spent() {
            built.projection = Some(projection);
        }
        rows
    }
}

/// Every hash-domain file on disk as a parsed document. Unreadable or
/// non-UTF-8 members are skipped — the projection serves what parses, exactly
/// as the resident projection does.
fn disk_docs(root: &fs::WorkspaceRoot) -> Result<model::Docs, String> {
    let domain = fs::domain::Domain::load(root).map_err(|e| format!("domain load: {e}"))?;
    let rels = fs::hash_domain(root, &domain).map_err(|e| format!("domain walk: {e}"))?;
    let mut docs = BTreeMap::new();
    for rel in rels {
        let Some(rel_str) = rel.to_str() else {
            continue;
        };
        let Ok(bytes) = std::fs::read_to_string(root.0.join(&rel)) else {
            continue;
        };
        docs.insert(rel_str.to_owned(), Arc::new(doc_of(rel_str, &bytes)));
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

#[cfg(test)]
mod tests {
    //! The reuse law of armed-plane § The ctx surface, over a counting
    //! backend: one projection per world, rebuilt only when pending bytes
    //! move or a call spent it, and every build sees exactly disk + pending.

    use std::cell::RefCell;

    use policy::{MwWorld as _, SqlValue};

    use super::{DoorWorld, SqlProjection, install_sql_backend};

    thread_local! {
        /// Every build on this thread: the `(path, bytes)` world it was handed.
        static BUILDS: RefCell<Vec<Vec<(String, String)>>> = const { RefCell::new(Vec::new()) };
    }

    /// Answers every query with the world it was built from, one row per doc;
    /// the query `spend` marks it spent.
    struct Recorded(Vec<(String, String)>, bool);

    impl SqlProjection for Recorded {
        fn query(&mut self, query: &str) -> Result<Vec<policy::SqlRow>, String> {
            self.1 |= query == "spend";
            Ok(self
                .0
                .iter()
                .map(|(path, raw)| {
                    vec![
                        ("path".to_owned(), SqlValue::Text(path.clone())),
                        ("raw".to_owned(), SqlValue::Text(raw.clone())),
                    ]
                })
                .collect())
        }

        fn spent(&self) -> bool {
            self.1
        }
    }

    #[allow(clippy::unnecessary_wraps)] // the `SqlBackend` signature
    fn recording(docs: &model::Docs) -> Result<Box<dyn SqlProjection>, String> {
        let world: Vec<(String, String)> = docs
            .iter()
            .map(|(path, doc)| (path.clone(), doc.raw.clone()))
            .collect();
        BUILDS.with(|b| b.borrow_mut().push(world.clone()));
        Ok(Box::new(Recorded(world, false)))
    }

    fn builds() -> usize {
        BUILDS.with(|b| b.borrow().len())
    }

    /// The `(path, raw)` rows one `ctx.sql` answers.
    fn world_of(world: &DoorWorld<'_>) -> Vec<(String, String)> {
        world
            .sql("SELECT path, raw")
            .expect("ctx.sql answers")
            .into_iter()
            .map(|row| match (&row[0].1, &row[1].1) {
                (SqlValue::Text(p), SqlValue::Text(r)) => (p.clone(), r.clone()),
                other => panic!("unexpected row {other:?}"),
            })
            .collect()
    }

    #[test]
    fn one_projection_per_world_and_every_build_reads_disk_plus_pending() {
        install_sql_backend(recording);
        let dir = tempfile::tempdir().expect("tempdir");
        let root = fs::WorkspaceRoot(dir.path().to_path_buf());
        std::fs::write(dir.path().join("a.md"), "# a on disk\n").unwrap();
        std::fs::write(dir.path().join("b.md"), "# b on disk\n").unwrap();

        let mut world = DoorWorld::new(&root);
        world.stage("a.md", "# a pending\n");
        let first = world_of(&world);
        assert_eq!(
            first,
            vec![
                ("a.md".to_owned(), "# a pending\n".to_owned()),
                ("b.md".to_owned(), "# b on disk\n".to_owned()),
            ],
            "pending bytes shadow disk"
        );
        assert_eq!(
            world_of(&world),
            first,
            "a second call reads the same world"
        );
        assert_eq!(builds(), 1, "two calls on one world build once");

        world.stage("a.md", "# a pending\n");
        world_of(&world);
        assert_eq!(
            builds(),
            1,
            "re-staging identical bytes keeps the projection"
        );

        // Disk moves under the run (a writer the flock does not exclude): the
        // run's world is still the disk it parsed on the first build.
        std::fs::write(dir.path().join("b.md"), "# b written later\n").unwrap();
        world.stage("a.md", "# a again\n");
        world.stage("c.md", "# c born\n");
        assert_eq!(
            world_of(&world),
            vec![
                ("a.md".to_owned(), "# a again\n".to_owned()),
                ("b.md".to_owned(), "# b on disk\n".to_owned()),
                ("c.md".to_owned(), "# c born\n".to_owned()),
            ],
            "moved pending bytes and a birth appear; a later disk write does not"
        );
        assert_eq!(builds(), 2, "moved pending bytes build once more");

        world.sql("spend").expect("a spending call still answers");
        assert_eq!(builds(), 2);
        world_of(&world);
        assert_eq!(builds(), 3, "the call after a spending one builds afresh");
    }
}
