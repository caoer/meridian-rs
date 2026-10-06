//! The link plane and claim-link colours over a generation whose bodies are
//! leased one at a time (merkle-spec §6.8): same answers as the eager corpus,
//! one live body, and a known member's fault is a refusal, never an absence.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::io;
use std::rc::Rc;
use std::sync::Arc;

use model::catalog::{DocumentCatalog, DocumentLease, EagerCatalog};
use wire::{ErrorCode, Path as WPath, PinSpec, Root};
use wire_serve::read::{links, page_decorations, page_decorations_in};
use wire_serve::write::{SpliceArgs, splice};

/// An eager corpus behind counted leases, with one member that can fault.
#[derive(Debug)]
struct Leased<'a> {
    eager: EagerCatalog<'a>,
    scans: RefCell<Vec<String>>,
    reads: Cell<usize>,
    live: Rc<Cell<usize>>,
    most_live: Rc<Cell<usize>>,
    fault: RefCell<Option<(String, Fault)>>,
}
#[derive(Debug, Clone, Copy)]
enum Fault {
    Backing,
    NoLease,
    OtherVersion,
}
impl<'a> Leased<'a> {
    fn new(docs: &'a model::Docs) -> Self {
        Self {
            eager: EagerCatalog::new(docs),
            scans: RefCell::default(),
            reads: Cell::new(0),
            live: Rc::default(),
            most_live: Rc::default(),
            fault: RefCell::default(),
        }
    }
    fn fail(&self, path: &str, fault: Fault) {
        *self.fault.borrow_mut() = Some((path.to_owned(), fault));
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
impl DocumentCatalog for Leased<'_> {
    fn revisions(&self) -> Box<dyn Iterator<Item = (&str, &model::NodeRev)> + '_> {
        self.eager.revisions()
    }
    fn revision(&self, path: &str) -> Option<&model::NodeRev> {
        self.eager.revision(path)
    }
    fn index(&self) -> &model::CorpusIndex {
        panic!("the caller supplies the generation's name index");
    }
    fn pin(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        self.scans.borrow_mut().push(path.to_owned());
        let mut served = path;
        if let Some((member, fault)) = &*self.fault.borrow()
            && member == path
        {
            match fault {
                Fault::Backing => return Err(io::Error::other("backing unavailable")),
                Fault::NoLease => return Ok(None),
                Fault::OtherVersion => {
                    served = self
                        .eager
                        .revisions()
                        .map(|(other, _)| other)
                        .find(|other| *other != path)
                        .expect("a second member");
                }
            }
        }
        Ok(self.eager.pin(served)?.map(|inner| {
            self.live.set(self.live.get() + 1);
            self.most_live
                .set(self.most_live.get().max(self.live.get()));
            Box::new(Lease {
                inner,
                live: self.live.clone(),
            }) as Box<dyn DocumentLease>
        }))
    }
    fn pin_read(&self, path: &str) -> io::Result<Option<Box<dyn DocumentLease + '_>>> {
        self.reads.set(self.reads.get() + 1);
        self.pin(path)
    }
}

fn corpus(files: &[(&str, &str)]) -> (model::CorpusIndex, model::Docs) {
    let mut index = model::CorpusIndex::new();
    let mut docs = model::Docs::new();
    for (path, raw) in files {
        let doc = model::build((*raw).to_owned(), syntax::parse(raw));
        index.insert(path, &doc);
        docs.insert((*path).to_owned(), Arc::new(doc));
    }
    (index, docs)
}

const LINKED: &[(&str, &str)] = &[
    ("a.md", "# A\n\n[[b]] [[b#Part]] ![[c]] [[nowhere]]\n"),
    ("b.md", "# B\n\n## Part\n\n[[a]] [[#Part]]\n"),
    ("c.md", "# C\n\nno edges\n"),
];

fn frame(
    root: &fs::WorkspaceRoot,
    index: &model::CorpusIndex,
    catalog: &dyn DocumentCatalog,
    unserved: &BTreeMap<String, String>,
    path: Option<&str>,
) -> Result<serde_json::Value, Box<wire::ErrorBody>> {
    let stamp = Root("b3b:selected-generation".to_owned());
    let live = stamp.clone();
    links(
        root,
        index,
        catalog,
        unserved,
        path.map(|p| WPath(p.to_owned())).as_ref(),
        stamp,
        0,
        || Ok(live),
    )
    .map(|body| serde_json::to_value(body).unwrap())
}

#[test]
fn the_edge_map_over_leases_equals_the_eager_map_and_holds_one_body() {
    let (index, docs) = corpus(LINKED);
    let leased = Leased::new(&docs);

    let whole = query::links_in(&index, &leased, None).unwrap();
    assert_eq!(whole, query::links(&index, &docs, None));
    assert_eq!(whole.len(), 3, "a link-less member keeps its empty entry");
    assert_eq!(leased.scanned(), ["a.md", "b.md", "c.md"]);
    assert_eq!(
        leased.most_live.get(),
        1,
        "a member's lease ends before the next member is admitted"
    );
    assert_eq!(leased.live.get(), 0);
    assert_eq!(leased.reads.get(), 0, "a scan is not an interactive read");

    let named = Leased::new(&docs);
    assert_eq!(
        query::links_in(&index, &named, Some("b.md")).unwrap(),
        query::links(&index, &docs, Some("b.md"))
    );
    assert_eq!(named.scanned(), ["b.md"], "a named page pins that page");

    let absent = Leased::new(&docs);
    assert!(
        query::links_in(&index, &absent, Some("ghost.md"))
            .unwrap()
            .is_empty()
    );
    assert!(absent.scanned().is_empty());
}

#[test]
fn a_known_member_that_cannot_be_leased_refuses_the_edge_map() {
    let (index, docs) = corpus(LINKED);
    for fault in [Fault::Backing, Fault::NoLease, Fault::OtherVersion] {
        let leased = Leased::new(&docs);
        leased.fail("b.md", fault);
        let refused = query::links_in(&index, &leased, None)
            .expect_err("a cold member is not an absent member");
        assert_eq!(refused.path, "b.md", "{fault:?}");
        assert_eq!(leased.live.get(), 0, "{fault:?}");
        // The members before it were served; nothing after it was admitted.
        assert_eq!(leased.scanned(), ["a.md", "b.md"], "{fault:?}");
    }
}

#[test]
fn the_links_frame_reads_membership_from_metadata_and_names_a_faulting_member() {
    let dir = tempfile::tempdir().unwrap();
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    for (path, raw) in LINKED {
        std::fs::write(dir.path().join(path), raw).unwrap();
    }
    std::fs::create_dir(dir.path().join(".hidden")).unwrap();
    std::fs::write(
        dir.path().join(".hidden/out.md"),
        "# Out\n\n[[a]] [[c#x]]\n",
    )
    .unwrap();
    let (index, docs) = corpus(LINKED);
    let unserved = BTreeMap::from([("bad.md".to_owned(), "is not valid UTF-8".to_owned())]);
    let eager = EagerCatalog::new(&docs);

    for path in [None, Some("a.md"), Some("c.md"), Some(".hidden/out.md")] {
        let leased = Leased::new(&docs);
        assert_eq!(
            frame(&root, &index, &leased, &unserved, path).unwrap(),
            frame(&root, &index, &eager, &unserved, path).unwrap(),
            "{path:?}"
        );
        assert!(leased.most_live.get() <= 1, "{path:?}");
        assert_eq!(leased.reads.get(), 0, "{path:?}");
        match path {
            None => assert_eq!(leased.scanned(), ["a.md", "b.md", "c.md"]),
            // A page outside the hash domain is read from disk; its edges
            // resolve from names alone.
            Some(".hidden/out.md") => assert!(leased.scanned().is_empty()),
            Some(member) => assert_eq!(leased.scanned(), [member]),
        }
    }

    let leased = Leased::new(&docs);
    for (path, code) in [
        ("bad.md", ErrorCode::InvalidUtf8),
        ("ghost.md", ErrorCode::FileNotFound),
    ] {
        let refused = frame(&root, &index, &leased, &unserved, Some(path)).unwrap_err();
        assert_eq!(refused.code, code, "{path}");
    }
    assert!(leased.scanned().is_empty());

    for path in [None, Some("b.md")] {
        let leased = Leased::new(&docs);
        leased.fail("b.md", Fault::Backing);
        let refused = frame(&root, &index, &leased, &unserved, path).unwrap_err();
        assert_eq!(refused.code, ErrorCode::IoError, "{path:?}");
        assert_eq!(refused.path, Some(WPath("b.md".to_owned())), "{path:?}");
        assert_eq!(
            refused.cause.as_deref(),
            Some("cannot read retained member b.md: backing unavailable"),
            "{path:?}"
        );
    }
}

/// A page whose claim link cites a pinned section of the guide, and one that
/// cites its own.
const PINNER: &str = "---\ntitle: Plan\n---\n\n# Plan\n\ndraws from [[guide#^leaders-guideline|Leader's Guideline]].\n";
const TARGET: &str =
    "# Guide\n\n## Leader's Guideline\n\nreview before you close.\n\n## Other\n\nunrelated.\n";
const SELF_PINNER: &str = "# Own\n\n## Rule\n\nkeep it.\n\n## Use\n\nsee [[#^rule]].\n";

fn pinned_workspace() -> (tempfile::TempDir, model::CorpusIndex, model::Docs) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("plan.md"), PINNER).expect("pinner");
    std::fs::write(dir.path().join("guide.md"), TARGET).expect("target");
    std::fs::write(dir.path().join("own.md"), SELF_PINNER).expect("self pinner");
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "scan@example.invalid"],
        vec!["config", "user.name", "scan"],
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(&args)
            .status()
            .expect("git runs in the test environment");
        assert!(status.success(), "git {args:?}");
    }
    let root = fs::WorkspaceRoot(dir.path().to_path_buf());
    for (page, target, selector) in [
        ("plan.md", "guide.md", "Guide/Leader's Guideline"),
        ("own.md", "own.md", "Own/Rule"),
    ] {
        let args = SpliceArgs {
            premises: Vec::new(),
            id: None,
            origin: wire_serve::guard::Origin::InProcess,
            path: WPath(page.into()),
            actor: None,
            now: None,
            receipt: None,
            if_root: None,
            dry: false,
            force: false,
            edits: Vec::new(),
            plan_edits: Vec::new(),
            pin: Some(PinSpec {
                target: WPath(target.into()),
                selector: wire::ReadSel::parse(selector),
                vibe: None,
                fingerprint: None,
                sec_rev: None,
            }),
            fields: BTreeMap::default(),
        };
        splice(&root, None, &args, &[], None).expect("the pin commits");
    }
    let mut index = model::CorpusIndex::new();
    let mut docs = model::Docs::new();
    for rel in ["plan.md", "guide.md", "own.md"] {
        let doc = fs::load(&root, std::path::Path::new(rel)).expect("the page loads");
        index.insert(rel, &doc);
        docs.insert(rel.to_owned(), Arc::new(doc));
    }
    (dir, index, docs)
}

#[test]
fn claim_link_colours_lease_the_lock_target_and_never_the_page() {
    let (_dir, index, docs) = pinned_workspace();

    let eager = page_decorations(&index, &docs, "plan.md");
    assert_eq!(eager.len(), 1, "the fixture decorates one claim link");
    let leased = Leased::new(&docs);
    let page = WPath("plan.md".to_owned());
    let got = page_decorations_in(&index, &leased, &page, &docs["plan.md"]).unwrap();
    assert_eq!(got, eager, "the colour is read off the target's body");
    assert_eq!(
        leased.scanned(),
        ["guide.md"],
        "the page is the caller's; only the lock target is leased"
    );
    assert_eq!(leased.most_live.get(), 1);
    assert_eq!(leased.live.get(), 0);
    assert_eq!(leased.reads.get(), 0, "a target is not an interactive read");

    // A page that pins its own section is classified against the document
    // the read already holds.
    let own = Leased::new(&docs);
    let page = WPath("own.md".to_owned());
    let eager = page_decorations(&index, &docs, "own.md");
    assert_eq!(eager.len(), 1, "the fixture decorates the self link");
    assert_eq!(
        page_decorations_in(&index, &own, &page, &docs["own.md"]).unwrap(),
        eager
    );
    assert!(own.scanned().is_empty());

    // A named page outside the generation decorates nothing and leases nothing.
    let outside = Leased::new(&docs);
    let page = WPath(".hidden/plan.md".to_owned());
    assert!(
        page_decorations_in(&index, &outside, &page, &docs["plan.md"])
            .unwrap()
            .is_empty()
    );
    assert!(outside.scanned().is_empty());
}

#[test]
fn a_lock_target_that_cannot_be_leased_refuses_the_read() {
    let (_dir, index, docs) = pinned_workspace();
    let page = WPath("plan.md".to_owned());
    for fault in [Fault::Backing, Fault::NoLease, Fault::OtherVersion] {
        let leased = Leased::new(&docs);
        leased.fail("guide.md", fault);
        let refused = page_decorations_in(&index, &leased, &page, &docs["plan.md"])
            .expect_err("a cold target is not a missing target");
        assert_eq!(refused.code, ErrorCode::IoError, "{fault:?}");
        assert_eq!(
            refused.path,
            Some(WPath("guide.md".to_owned())),
            "{fault:?}"
        );
        assert_eq!(leased.live.get(), 0, "{fault:?}");
    }
}
