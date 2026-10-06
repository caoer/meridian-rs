//! The walk over a generation whose bodies are leased one at a time
//! (merkle-spec §6.8): the eager walk's answer, one live body, only lock
//! holders and edge targets read, and a known member's fault is a refusal.

use std::cell::{Cell, RefCell};
use std::io;
use std::rc::Rc;
use std::sync::Arc;

use model::catalog::{DocumentLease, EagerCatalog};

use super::*;

/// Live leases across every catalog of one walk.
#[derive(Debug, Default, Clone)]
struct Gauge {
    live: Rc<Cell<usize>>,
    most: Rc<Cell<usize>>,
}

#[derive(Debug, Clone, Copy)]
enum Fault {
    Backing,
    NoLease,
    OtherVersion,
}

/// An eager corpus behind counted leases, with one member that can fault.
#[derive(Debug)]
struct Counted<'a> {
    docs: &'a model::Docs,
    eager: EagerCatalog<'a>,
    scans: RefCell<Vec<String>>,
    gauge: Gauge,
    fault: Option<(&'static str, Fault)>,
}

impl<'a> Counted<'a> {
    fn new(docs: &'a model::Docs, gauge: &Gauge) -> Self {
        Self {
            docs,
            eager: EagerCatalog::new(docs),
            scans: RefCell::default(),
            gauge: gauge.clone(),
            fault: None,
        }
    }
    fn scanned(&self) -> Vec<String> {
        self.scans.borrow().clone()
    }
}

#[derive(Debug)]
struct Lease<'a> {
    inner: Box<dyn DocumentLease + 'a>,
    live: Rc<Cell<usize>>,
}
impl DocumentLease for Lease<'_> {
    fn document(&self) -> io::Result<&model::Document> {
        self.inner.document()
    }
}
impl Drop for Lease<'_> {
    fn drop(&mut self) {
        self.live.set(self.live.get() - 1);
    }
}

impl DocumentCatalog for Counted<'_> {
    fn revisions(&self) -> Box<dyn Iterator<Item = (&str, &model::NodeRev)> + '_> {
        self.eager.revisions()
    }
    fn revision(&self, path: &str) -> Option<&model::NodeRev> {
        self.eager.revision(path)
    }
    fn index(&self) -> &model::CorpusIndex {
        self.eager.index()
    }
    fn link_roots(&self) -> BTreeSet<addr::MountName> {
        self.eager.link_roots()
    }
    fn pin(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        self.scans.borrow_mut().push(path.to_owned());
        let mut served = path;
        if let Some((member, fault)) = self.fault
            && member == path
        {
            match fault {
                Fault::Backing => return Err(io::Error::other("backing unavailable")),
                Fault::NoLease => return Ok(None),
                Fault::OtherVersion => {
                    served = self
                        .docs
                        .keys()
                        .map(String::as_str)
                        .find(|other| *other != path)
                        .expect("a second member");
                }
            }
        }
        Ok(self.eager.pin(served)?.map(|inner| {
            self.gauge.live.set(self.gauge.live.get() + 1);
            self.gauge
                .most
                .set(self.gauge.most.get().max(self.gauge.live.get()));
            Box::new(Lease {
                inner,
                live: self.gauge.live.clone(),
            }) as Box<dyn DocumentLease>
        }))
    }
}

impl LockFacts for Counted<'_> {
    fn lock_roots(&self) -> BTreeSet<addr::MountName> {
        lock_addressed_roots(self.docs)
    }
    fn lock_holders(&self) -> Box<dyn Iterator<Item = &str> + '_> {
        Box::new(
            self.docs
                .iter()
                .filter(|(_, doc)| lock_fact(doc).is_some())
                .map(|(path, _)| path.as_str()),
        )
    }
}

fn doc(raw: &str) -> Arc<model::Document> {
    Arc::new(model::build(raw.to_owned(), syntax::parse(raw)))
}

fn token(raw: &str) -> String {
    let d = doc(raw);
    model::fingerprint::fingerprint(&d, &d.root)
        .expect("the fixture page has content")
        .into_string()
}

/// A page whose lock pins the whole body of each `(object, token)`.
fn pinning(title: &str, pins: &[(&str, &str)]) -> String {
    let mut l = lock::Lock::new();
    for (object, token) in pins {
        l.upsert_pin(lock::PinEntry::new(
            object,
            "9ae3f1deadbeef",
            lock::Selector::Path(Vec::new()),
            token,
        ));
    }
    format!("# {title}\n\n{}\n", lock::render(&l))
}

fn mount() -> addr::MountName {
    addr::MountName::parse("r").unwrap()
}

/// Ambient `a → b → c`, `a → r:remote`, `d → b`, and `plain` with no lock;
/// the mounted root `r` holds `remote` and `other`. Every pin is live.
fn worlds() -> (model::Docs, model::Docs) {
    let remote_raw = "# Remote\n\nmounted body\n";
    let c_raw = "# C\n\nleaf body\n";
    let b_raw = pinning("B", &[("c", &token(c_raw))]);
    let a_raw = pinning(
        "A",
        &[("b", &token(&b_raw)), ("r:remote", &token(remote_raw))],
    );
    let d_raw = pinning("D", &[("b", &token(&b_raw))]);
    let ambient = [
        ("a.md", a_raw.as_str()),
        ("b.md", b_raw.as_str()),
        ("c.md", c_raw),
        ("d.md", d_raw.as_str()),
        ("plain.md", "# Plain\n\nno lock\n"),
    ]
    .into_iter()
    .map(|(path, raw)| (path.to_owned(), doc(raw)))
    .collect();
    let mounted = [
        ("other.md", "# Other\n\nunpinned\n"),
        ("remote.md", remote_raw),
    ]
    .into_iter()
    .map(|(path, raw)| (path.to_owned(), doc(raw)))
    .collect();
    (ambient, mounted)
}

#[test]
fn the_leased_walk_equals_the_eager_walk_and_reads_only_holders_and_targets() {
    let (docs, remote) = worlds();
    let mounts = addr::MountSet::new([mount()]);
    let eager_corpus = model::RootedCorpus::ambient(&docs).with_root(mount(), &remote);

    for (root, direction, depth) in [
        ("a.md", Direction::Up, None),
        ("c.md", Direction::Down, None),
        ("b.md", Direction::Down, Some(1)),
        ("plain.md", Direction::Up, None),
    ] {
        let case = format!("{root} {direction:?} {depth:?}");
        let gauge = Gauge::default();
        let ambient = Counted::new(&docs, &gauge);
        let mounted = Counted::new(&remote, &gauge);
        let corpus = RootedCatalog::ambient(&ambient).with_root(mount(), &mounted);

        let got = walk_leased(&corpus, &ambient, &mounts, root, direction, depth).unwrap();
        let eager = walk_rooted(&eager_corpus, &mounts, root, direction, depth).unwrap();
        assert_eq!(got, eager, "{case}");
        assert!(
            !ambient.scanned().contains(&"plain.md".to_owned()),
            "{case}: a member with no lock row that is no edge's target is never read"
        );
        assert_eq!(gauge.most.get(), 1, "{case}: one body at a time");
        assert_eq!(gauge.live.get(), 0, "{case}");
        if root == "a.md" {
            assert_eq!(
                got.entries.len(),
                3,
                "the fixture reaches b, r:remote and c"
            );
            assert!(
                got.entries.iter().all(|e| e.color == Color::Green),
                "every fixture pin is live: {:?}",
                got.entries
            );
            assert_eq!(
                mounted.scanned(),
                ["remote.md"],
                "a mounted root's target is leased from that root"
            );
        }
    }
}

#[test]
fn a_known_member_that_cannot_be_leased_refuses_the_walk() {
    let (docs, remote) = worlds();
    let mounts = addr::MountSet::new([mount()]);

    // A lock holder, an ambient edge target, and a mounted root's target.
    for (member, in_mount, named) in [
        ("b.md", false, "b.md"),
        ("c.md", false, "c.md"),
        ("remote.md", true, "r:remote.md"),
    ] {
        for fault in [Fault::Backing, Fault::NoLease, Fault::OtherVersion] {
            let case = format!("{member} {fault:?}");
            let gauge = Gauge::default();
            let mut ambient = Counted::new(&docs, &gauge);
            let mut mounted = Counted::new(&remote, &gauge);
            if in_mount {
                mounted.fault = Some((member, fault));
            } else {
                ambient.fault = Some((member, fault));
            }
            let corpus = RootedCatalog::ambient(&ambient).with_root(mount(), &mounted);

            let stop = walk_leased(&corpus, &ambient, &mounts, "a.md", Direction::Up, None)
                .expect_err("a cold member is not an absent member");
            let WalkStop::Fault(refused) = stop else {
                panic!("{case}: {stop:?}");
            };
            assert_eq!(refused.path, named, "{case}");
            assert_eq!(gauge.live.get(), 0, "{case}");
        }
    }
}

/// A lock holder that cannot be leased refuses the walk even when no edge
/// would ever colour it: `d.md` pins `b.md` and nothing pins `d.md`, so on a
/// down walk from `b.md` it is read for its pins alone. Skipping it would
/// answer a blast radius with one dependent missing.
#[test]
fn a_lock_holder_no_edge_targets_still_refuses_when_it_cannot_be_leased() {
    let (docs, remote) = worlds();
    let mounts = addr::MountSet::new([mount()]);
    for fault in [Fault::Backing, Fault::NoLease, Fault::OtherVersion] {
        let gauge = Gauge::default();
        let mut ambient = Counted::new(&docs, &gauge);
        let mounted = Counted::new(&remote, &gauge);
        ambient.fault = Some(("d.md", fault));
        let corpus = RootedCatalog::ambient(&ambient).with_root(mount(), &mounted);

        let stop = walk_leased(&corpus, &ambient, &mounts, "b.md", Direction::Down, None)
            .expect_err("a holder that cannot be read is not a holder with no pins");
        let WalkStop::Fault(refused) = stop else {
            panic!("{fault:?}: {stop:?}");
        };
        assert_eq!(refused.path, "d.md", "{fault:?}");
        assert_eq!(gauge.live.get(), 0, "{fault:?}");
    }
}
