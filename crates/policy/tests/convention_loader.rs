//! U1.3 merge-gate fixtures — the four named loader acceptances, exercised through
//! the PUBLIC `policy` surface:
//!
//! 1. a valid convention loads;
//! 2. a FIX file refuses with deferral text (the capability ceiling, load-bearing
//!    negative — shown FAILING);
//! 3. an out-of-scope path is not matched;
//! 4. the seed convention fires on its (self-close) scenario and passes on its
//!    (reviewer-close) sibling — the refusal cites the passing scenario.

use std::collections::BTreeMap;

use model::{Document, NodeKind};
use policy::{
    Capability, Change, ChangeOp, CheckLimits, ConventionFiles, Invocation, LoadError,
    SEED_CONVENTION_SLUG, derive_change, load_convention, load_seed_convention,
};

/// An in-memory convention folder.
struct MemFiles(BTreeMap<String, String>);

impl MemFiles {
    fn new() -> Self {
        Self(BTreeMap::new())
    }
    fn with(mut self, rel: &str, body: &str) -> Self {
        self.0.insert(rel.to_string(), body.to_string());
        self
    }
}

impl ConventionFiles for MemFiles {
    fn read(&self, rel_path: &str) -> std::io::Result<String> {
        self.0.get(rel_path).cloned().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, format!("no {rel_path}"))
        })
    }
    fn exists(&self, rel_path: &str) -> bool {
        self.0.contains_key(rel_path)
    }
}

const VALID_CHECK: &str = "\
---
paths:
  - tasks/**
---

# demo

```starlark
def check_change(change):
    pass
```
";

/// Fixture 1 — a well-formed convention loads, with its declared scope.
#[test]
fn fixture_valid_convention_loads() {
    let files = MemFiles::new().with("CHECK.md", VALID_CHECK);
    let conv = load_convention("demo", &files, CheckLimits::default())
        .expect("a well-formed convention loads");
    assert_eq!(conv.slug(), "demo");
    assert_eq!(conv.scope(), &["tasks/**".to_string()]);
}

/// Fixture 2 — a convention declaring a FIX file is REFUSED (v1 ships CHECK only),
/// with a deferral message naming the capability. Load-bearing negative: the load
/// MUST fail, and the refusal text MUST name the deferral.
#[test]
fn fixture_fix_file_refuses_with_deferral_text() {
    let files = MemFiles::new()
        .with("CHECK.md", VALID_CHECK)
        .with("FIX.md", "# a deferred fix\n");
    let err = load_convention("demo", &files, CheckLimits::default())
        .expect_err("declaring FIX must refuse — v1 ships CHECK only");
    assert_eq!(
        err,
        LoadError::CapabilityDeferred {
            capability: Capability::Fix
        }
    );
    let text = err.to_string();
    assert!(
        text.contains("FIX"),
        "the refusal names the capability: {text}"
    );
    assert!(
        text.contains("v1 ships CHECK only"),
        "the refusal names the deferral: {text}"
    );
}

/// Fixture 3 — a document path outside the `paths:` scope is not matched.
#[test]
fn fixture_out_of_scope_path_not_matched() {
    let files = MemFiles::new().with("CHECK.md", VALID_CHECK);
    let conv = load_convention("demo", &files, CheckLimits::default()).unwrap();
    assert!(conv.matches_path("tasks/fix-parser.md"), "in scope");
    assert!(
        !conv.matches_path("notes/plan.md"),
        "out-of-scope path is not the convention's concern"
    );
}

/// Build a real `model::Document`, stamping its path (the disk edge's job).
fn doc_of(path: &str, md: &str) -> Document {
    let nodes = syntax::parse(md);
    let mut doc = model::build(md.to_string(), nodes);
    if let NodeKind::Document { path: p, .. } = &mut doc.root.kind {
        *p = path.to_string();
    }
    doc
}

fn no_edges(_: &str) -> Option<(String, Document)> {
    None
}

/// A close of `tasks/fix-parser.md` by `actor` (matches the seed scenario base).
fn close_change(actor: &str) -> Change {
    let before = doc_of(
        "tasks/fix-parser.md",
        "---\nowner: agent:alice\nstatus: open\n---\n# Fix parser\n\nx\n",
    );
    let after = doc_of(
        "tasks/fix-parser.md",
        "---\nowner: agent:alice\nstatus: closed\n---\n# Fix parser\n\nx\n",
    );
    derive_change(
        &before,
        &after,
        &[],
        Invocation {
            op: ChangeOp::Splice,
            actor: Some(actor),
            force: false,
        },
        &[],
        &no_edges,
    )
}

/// Fixture 4 — the seed convention loads and fires on its scenario. The owner
/// closing their own task fires (a refusal citing the passing scenario); a distinct
/// reviewer passes.
#[test]
fn fixture_seed_convention_fires_on_its_scenario() {
    let seed =
        load_seed_convention(CheckLimits::default()).expect("the throwaway seed convention loads");
    assert_eq!(seed.slug(), SEED_CONVENTION_SLUG);
    assert!(
        seed.matches_path("tasks/fix-parser.md"),
        "the scenario is in scope"
    );

    // owner-self-close: agent:alice IS the owner → fires.
    let self_close = seed.check_change(&close_change("agent:alice")).unwrap();
    assert!(self_close.fired(), "owner self-close fires");
    assert_eq!(self_close.refusals.len(), 1);
    assert!(
        self_close.refusals[0].message.contains("agent:alice"),
        "the refusal teaches who over-reached"
    );
    assert_eq!(
        self_close.refusals[0].passing_scenario, "scenarios/reviewer-close.md",
        "the refusal cites the passing scenario (the legal path)"
    );

    // reviewer-close: agent:bob is a distinct reviewer → passes.
    let reviewer_close = seed.check_change(&close_change("agent:bob")).unwrap();
    assert!(!reviewer_close.fired(), "reviewer != owner passes");
}
