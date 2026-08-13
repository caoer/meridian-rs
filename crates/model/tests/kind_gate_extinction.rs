//! The kind gate's EXTINCTION pin (kind-sweep, ZT 2026-08-13; the daemon's
//! mountgate carries the same pin on its side since 2026-08-12).
//!
//! `kind` left the schema: mounts have no taxonomy, every mounted root is the
//! same shape to the resolver, and its documents are parsed wherever they
//! came from — so a `#selector` into a mount that would once have been
//! `git-folder` resolves exactly like any other rooted address. The old
//! `SelectorOnOpaqueRoot` refusal refused work the corpus could do (the
//! index was built identically for both kinds). A re-added kind gate fails
//! here first.

use addr::{MountName, MountSet};
use model::{CorpusIndex, Document, RefResolution, RootedCorpus};
use std::collections::BTreeMap;

fn name(n: &str) -> MountName {
    MountName::parse(n).expect("a canonical root name")
}

fn corpus_with(paths: &[&str]) -> BTreeMap<String, Document> {
    let mut docs = BTreeMap::new();
    for p in paths {
        docs.insert(
            (*p).to_owned(),
            model::build(String::new(), syntax::parse("")),
        );
    }
    docs
}

#[test]
fn a_selector_into_any_mounted_root_resolves_like_any_rooted_address() {
    let ambient = corpus_with(&["notes.md"]);
    let repo = corpus_with(&["src/main.rs"]);
    let corpus = RootedCorpus::ambient(&ambient).with_root(name("repo"), &repo);
    let mounts = MountSet::new([name("repo")]);

    let got =
        CorpusIndex::new().resolve_ref("repo:src/main.rs#impl-notes", "claim.md", &corpus, &mounts);
    assert_eq!(
        got,
        RefResolution::Rooted {
            root: name("repo"),
            path: "src/main.rs".to_owned(),
        },
        "no mount taxonomy exists to gate on — the resolver answers with the file",
    );
}
