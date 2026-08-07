//! Kind-gating at the rooted resolver (wire-map W25, § 10.1 G-1): `vault`
//! roots parse and take selectors; an opaque root (`git-folder`) has no parse
//! and no sections, so a `#selector` into one is a resolution-time refusal —
//! never a parse error, never a silent drop of the fragment.
//!
//! The arm under test sits between mount binding and path lookup
//! (`resolve_ref`'s opaque check), so the refusal fires on the ADDRESS SHAPE,
//! before the corpus is consulted.

use addr::{AddrError, MountName, MountSet};
use model::{CorpusIndex, Document, RefResolution, RootKind, RootedCorpus};
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
fn a_selector_into_a_git_folder_root_refuses_with_the_opaque_class() {
    let ambient = corpus_with(&["notes.md"]);
    let repo = corpus_with(&["src/main.rs"]);
    let corpus = RootedCorpus::ambient(&ambient).with_root(
        name("repo"),
        RootKind::Opaque("git-folder".to_owned()),
        &repo,
    );
    let mounts = MountSet::new([name("repo")]);

    let got = CorpusIndex::new().resolve_ref("repo:src/main.rs#impl-notes", "claim.md", &corpus, &mounts);
    assert_eq!(
        got,
        RefResolution::Malformed(AddrError::SelectorOnOpaqueRoot {
            root: name("repo"),
            kind: "git-folder".to_owned(),
            selector: "impl-notes".to_owned(),
        }),
        "a structured selector into an opaque root is the G-1 refusal — \
         carrying the root, its kind word, and the refused selector verbatim",
    );

    // The refusal text names the right locus: the root, its kind, the selector,
    // and the file-grain fix — not a parse complaint, not file-not-found.
    let RefResolution::Malformed(err) = got else {
        unreachable!()
    };
    let text = err.to_string();
    for needle in ["'repo'", "git-folder", "'impl-notes'", "no parse and no sections"] {
        assert!(
            text.contains(needle),
            "the refusal must name {needle}: {text}",
        );
    }
}

/// The gate is kind-gating, not selector-hostility: the SAME selector into a
/// `vault` root resolves. A build that refused every selector would pass the
/// refusal test above; this half catches it.
#[test]
fn the_same_selector_into_a_vault_root_still_resolves() {
    let ambient = corpus_with(&["notes.md"]);
    let wiki = corpus_with(&["plan.md"]);
    let corpus =
        RootedCorpus::ambient(&ambient).with_root(name("wiki"), RootKind::Vault, &wiki);
    let mounts = MountSet::new([name("wiki")]);

    let got = CorpusIndex::new().resolve_ref("wiki:plan.md#impl-notes", "claim.md", &corpus, &mounts);
    assert_eq!(
        got,
        RefResolution::Rooted {
            root: name("wiki"),
            path: "plan.md".to_owned(),
        },
        "vault roots have sections — the selector must not trip the opaque gate",
    );
}

/// The gate is selector-gating, not root-hostility: the same opaque root
/// WITHOUT a selector resolves to the file. File-grain addressing into a
/// `git-folder` root is the supported spelling the refusal teaches.
#[test]
fn the_same_git_folder_root_without_a_selector_still_resolves_to_the_file() {
    let ambient = corpus_with(&["notes.md"]);
    let repo = corpus_with(&["src/main.rs"]);
    let corpus = RootedCorpus::ambient(&ambient).with_root(
        name("repo"),
        RootKind::Opaque("git-folder".to_owned()),
        &repo,
    );
    let mounts = MountSet::new([name("repo")]);

    let got = CorpusIndex::new().resolve_ref("repo:src/main.rs", "claim.md", &corpus, &mounts);
    assert_eq!(
        got,
        RefResolution::Rooted {
            root: name("repo"),
            path: "src/main.rs".to_owned(),
        },
        "file-grain addressing into an opaque root is legal — only sections are not",
    );
}

/// Resolution-time, before path lookup: the refusal fires on the address
/// shape even when the named path does not exist in the root — the answer is
/// the selector refusal, never file-not-found with a silently dropped fragment.
#[test]
fn the_opaque_refusal_outranks_file_not_found() {
    let ambient = corpus_with(&["notes.md"]);
    let repo = corpus_with(&["src/main.rs"]);
    let corpus = RootedCorpus::ambient(&ambient).with_root(
        name("repo"),
        RootKind::Opaque("git-folder".to_owned()),
        &repo,
    );
    let mounts = MountSet::new([name("repo")]);

    let got = CorpusIndex::new().resolve_ref("repo:no/such/file.rs#sec", "claim.md", &corpus, &mounts);
    assert!(
        matches!(
            &got,
            RefResolution::Malformed(AddrError::SelectorOnOpaqueRoot { .. })
        ),
        "the kind gate answers before the corpus is consulted; got {got:?}",
    );
}
