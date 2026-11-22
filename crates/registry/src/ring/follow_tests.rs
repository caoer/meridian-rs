//! The follow step's equivalence gate (merkle-spec §6.1, §6.7): over a
//! randomized history of write-door commits, edits made behind the engine,
//! reader observations, detect cycles and primes, a ring whose follow step
//! classifies the overlay journal's movers tells the frames, and keeps the
//! baseline roots, of a ring that classifies the memo's whole leaf view.

use super::*;
use std::collections::BTreeMap;
use std::path::Path;
use wire_serve::write::{CreateArgs, RemoveArgs, ResidentDoor, SpliceArgs};

const DOMAIN_CONFIG: &str = "---\nignore:\n  - \"drafts/**\"\n---\n# Domain\n";

/// xorshift64: the history is a pure function of the seed.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        usize::try_from(self.0 % u64::try_from(n).unwrap()).unwrap()
    }
}

/// One workspace under one registry, and what the test knows of its files:
/// each managed file's version, which its body spells.
struct World {
    _dir: tempfile::TempDir,
    root: fs::WorkspaceRoot,
    registry: crate::Registry,
    ring: WorkspaceRing,
    whole: bool,
    files: BTreeMap<String, u32>,
    born: usize,
    delivered: u64,
    journal_follows: usize,
    trace: Vec<String>,
}

fn body(rel: &str, version: u32) -> String {
    format!("# {rel}\n\n- item v{version} ^t1\n")
}

impl World {
    fn new(whole: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("ws");
        let mut files = BTreeMap::new();
        for rel in [
            "a.md",
            "notes/b.md",
            "notes/c.md",
            "drafts/x.md",
            "drafts/y.md",
        ] {
            let path = ws.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body(rel, 0)).unwrap();
            files.insert(rel.to_owned(), 0);
        }
        let mut config = crate::Config::for_cache_root(dir.path().join("cache"));
        config.socket_path = dir.path().join("daemon.sock");
        config.state_path = dir.path().join("state.json");
        let registry = crate::in_process_registry(&config).unwrap();
        let root = fs::WorkspaceRoot(std::fs::canonicalize(ws).unwrap());
        let mut world = World {
            ring: WorkspaceRing::new(&root),
            _dir: dir,
            root,
            registry,
            whole,
            files,
            born: 0,
            delivered: 0,
            journal_follows: 0,
            trace: Vec::new(),
        };
        world.fresh_ring();
        world.ring.prime(&world.root, &world.registry).unwrap();
        world
    }

    /// A new epoch: the subscriber left and another arrives, unprimed.
    fn fresh_ring(&mut self) {
        self.journal_follows += self.ring.journal_follows.load(Ordering::SeqCst);
        self.ring = WorkspaceRing::new(&self.root);
        self.ring.follow_whole.store(self.whole, Ordering::SeqCst);
        self.delivered = 0;
    }

    /// One write door call, as the server's arms make it: the workspace
    /// memo, a [`DoorPlane`] as the sink and as the observation.
    fn door<T>(
        &self,
        call: impl FnOnce(&DoorPlane<'_>, ResidentDoor<'_>) -> Result<T, Box<ErrorBody>>,
    ) -> Result<(), ErrorCode> {
        let cache = self.registry.domain_cache(&self.root.0);
        let plane = DoorPlane::new(&self.registry, &self.root, &self.ring, &cache);
        let observe = || plane.observe();
        let supplied = ResidentDoor {
            cache: &cache,
            observe: &observe,
        };
        call(&plane, supplied).map(|_| ()).map_err(|e| e.code)
    }

    fn put(&mut self, rel: &str) -> Result<(), ErrorCode> {
        let version = self.files[rel];
        let args = SpliceArgs {
            premises: Vec::new(),
            id: None,
            path: wire::Path(rel.to_owned()),
            origin: wire_serve::guard::Origin::Wire,
            actor: Some("alice".to_owned()),
            now: Some("2026-08-15T00:00:00Z".to_owned()),
            receipt: None,
            if_root: None,
            dry: false,
            force: true,
            edits: vec![wire::Edit {
                target: wire::SecRef::Anchor {
                    anchor: "t1".to_owned(),
                },
                edit: wire::EditShape::Match {
                    old: format!("v{version}"),
                    new: format!("v{}", version + 1),
                },
                if_node_rev: None,
            }],
            plan_edits: Vec::new(),
            pin: None,
            fields: BTreeMap::default(),
        };
        let done = self.door(|plane, supplied| {
            wire_serve::write::splice(&self.root, Some(plane), &args, &[], Some(supplied))
        });
        if done.is_ok() {
            self.files.insert(rel.to_owned(), version + 1);
        }
        done
    }

    fn create(&mut self, rel: &str, body: String) -> Result<(), ErrorCode> {
        let args = CreateArgs {
            id: None,
            path: wire::Path(rel.to_owned()),
            body,
            actor: Some("alice".to_owned()),
            now: Some("2026-08-15T00:00:00Z".to_owned()),
            if_root: None,
            dry: false,
            fields: BTreeMap::default(),
            props: BTreeMap::default(),
        };
        self.door(|plane, supplied| {
            wire_serve::write::create_with_cache(
                &self.root,
                Some(plane),
                &args,
                &[],
                Some(supplied),
            )
        })
    }

    fn remove(&mut self, rel: &str) -> Result<(), ErrorCode> {
        let rev = fs::load(&self.root, Path::new(rel))
            .unwrap()
            .root
            .node_rev
            .0;
        let args = RemoveArgs {
            id: None,
            path: wire::Path(rel.to_owned()),
            if_file_rev: Some(wire::NodeRev(rev)),
            actor: Some("alice".to_owned()),
            now: Some("2026-08-15T00:00:00Z".to_owned()),
            if_root: None,
            dry: false,
        };
        self.door(|plane, supplied| {
            wire_serve::write::remove_with_cache(
                &self.root,
                Some(plane),
                &args,
                &[],
                Some(supplied),
            )
        })
    }

    fn some_file(&self, rng: &mut Rng) -> Option<String> {
        let at = rng.below(self.files.len().max(1));
        self.files.keys().nth(at).cloned()
    }

    fn new_file(&mut self) -> String {
        self.born += 1;
        let dir = ["", "notes/", "drafts/"][self.born % 3];
        format!("{dir}n{}.md", self.born)
    }

    /// One step of the history, chosen by `rng` from what the test knows.
    fn step(&mut self, rng: &mut Rng) {
        let what = match rng.below(16) {
            choice @ 0..=7 => self.through_a_door(choice, rng),
            choice @ 8..=11 => self.behind_the_engine(choice, rng),
            12 => {
                let cache = self.registry.domain_cache(&self.root.0);
                observe(&self.registry, &self.root, &cache);
                "a reader observes".to_owned()
            }
            13 | 14 => format!(
                "detect cycle: {:?}",
                self.ring
                    .cycle_now(&self.root, &self.registry)
                    .map_err(|e| e.code)
            ),
            _ if rng.below(2) == 0 => {
                self.fresh_ring();
                "the subscriber leaves".to_owned()
            }
            _ => format!(
                "prime: {:?}",
                self.ring
                    .prime(&self.root, &self.registry)
                    .map_err(|e| e.code)
            ),
        };
        self.record(&what);
    }

    /// A commit through a write door: a put, a birth, a death, or the domain
    /// config landing or leaving (the membership overlay).
    fn through_a_door(&mut self, choice: usize, rng: &mut Rng) -> String {
        let config = fs::domain::DOMAIN_CONFIG_PATH;
        match choice {
            0..=4 => match self.some_file(rng) {
                Some(rel) => format!("put {rel}: {:?}", self.put(&rel)),
                None => "put: no file".to_owned(),
            },
            5 => {
                let rel = self.new_file();
                let born = self.create(&rel, body(&rel, 0));
                if born.is_ok() {
                    self.files.insert(rel.clone(), 0);
                }
                format!("create {rel}: {born:?}")
            }
            6 => match self.some_file(rng) {
                Some(rel) => {
                    let gone = self.remove(&rel);
                    if gone.is_ok() {
                        self.files.remove(&rel);
                    }
                    format!("remove {rel}: {gone:?}")
                }
                None => "remove: no file".to_owned(),
            },
            _ if self.root.0.join(config).exists() => {
                format!("config remove: {:?}", self.remove(config))
            }
            _ => format!(
                "config create: {:?}",
                self.create(config, DOMAIN_CONFIG.to_owned())
            ),
        }
    }

    /// A change no door made: a rewrite, a birth or an unlink on disk.
    fn behind_the_engine(&mut self, choice: usize, rng: &mut Rng) -> String {
        if choice == 10 {
            let rel = self.new_file();
            let path = self.root.0.join(&rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body(&rel, 0)).unwrap();
            self.files.insert(rel.clone(), 0);
            return format!("birth behind the engine {rel}");
        }
        let Some(rel) = self.some_file(rng) else {
            return "behind the engine: no file".to_owned();
        };
        if choice == 11 {
            std::fs::remove_file(self.root.0.join(&rel)).unwrap();
            self.files.remove(&rel);
            return format!("unlink behind the engine {rel}");
        }
        let version = self.files[&rel] + 1;
        std::fs::write(self.root.0.join(&rel), body(&rel, version)).unwrap();
        self.files.insert(rel.clone(), version);
        format!("edit behind the engine {rel}")
    }

    /// The step's observable outcome: the frames it put on the ring and the
    /// baseline root it left.
    fn record(&mut self, what: &str) {
        let frames = self.ring.frames_after(self.delivered);
        if let Some(last) = frames.last() {
            self.delivered = last.delta.seq;
        }
        let baseline = self.ring.state().watch.root().cloned();
        self.trace.push(format!(
            "{what}\n  frames {frames:#?}\n  baseline {baseline:?}"
        ));
    }
}

/// One seeded history under one follow step; returns its trace and how many
/// follow steps classified the journal's movers.
fn history(seed: u64, whole: bool) -> (Vec<String>, usize) {
    let mut world = World::new(whole);
    let mut rng = Rng(seed);
    for _ in 0..48 {
        world.step(&mut rng);
    }
    // Close on a floor cycle over an edit behind the engine: it classifies
    // the memo's whole leaf view against the baseline, so a baseline member
    // an earlier follow step left stale is told here.
    world.ring.prime(&world.root, &world.registry).unwrap();
    std::fs::write(world.root.0.join("closing.md"), "# Closing\n").unwrap();
    let told = world.ring.cycle_now(&world.root, &world.registry);
    world.record(&format!("closing cycle: {:?}", told.map_err(|e| e.code)));
    world.fresh_ring();
    (world.trace, world.journal_follows)
}

#[test]
fn a_follow_step_on_the_journal_tells_what_the_whole_leaf_view_tells() {
    let mut journal_follows = 0;
    for seed in [1, 2, 3, 5, 8, 13, 21, 34] {
        let (by_journal, taken) = history(seed, false);
        let (by_leaf_view, untaken) = history(seed, true);
        assert_eq!(untaken, 0, "the oracle classifies the whole leaf view");
        for (step, (journal, whole)) in by_journal.iter().zip(&by_leaf_view).enumerate() {
            assert_eq!(journal, whole, "seed {seed}, step {step}");
        }
        assert_eq!(by_journal.len(), by_leaf_view.len(), "seed {seed}");
        journal_follows += taken;
    }
    assert!(
        journal_follows >= 40,
        "the histories must exercise the journal's follow step, took it {journal_follows} times"
    );
}
