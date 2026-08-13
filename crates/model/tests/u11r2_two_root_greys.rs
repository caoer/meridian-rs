//! Two root greys are distinct causes/words at the resolver (S3-R43 / S3-R49).
//!
//! Declared-but-unreadable is unreachable via `mrd walk` (one pass builds
//! `MountSet` + corpora together), so the disagreeing pair — name declared,
//! corpus absent (§ 8 M6) — is constructed here; without it the arm would
//! compile unexercised. Collapsing both arms to `Unmounted` yields a false
//! "declare it" teaching on an already-declared root.

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
fn a_root_nothing_declares_is_unmounted_and_teaches_the_declaration() {
    let ambient = corpus_with(&["notes.md"]);
    let corpus = RootedCorpus::ambient(&ambient);
    let index = CorpusIndex::new();

    let got = index.resolve_ref(
        "sessions:notes.md",
        "claim.md",
        &corpus,
        &MountSet::default(),
    );
    assert_eq!(
        got,
        RefResolution::Unmounted(name("sessions")),
        "nothing declares `sessions`, so the fix IS to declare it",
    );

    let teaching = model::selector::render_unmounted(&name("sessions"), "sessions:notes.md");
    assert!(
        teaching.contains("Fix: declare 'sessions' in ~/MERIDIAN.md"),
        "arm (a) teaches the declaration: {teaching}",
    );
}

/// The `MountSet` and `RootedCorpus` deliberately disagree — the pairing
/// production cannot yet produce.
#[test]
fn a_declared_but_unreadable_root_is_path_unseeable_and_teaches_the_path() {
    let ambient = corpus_with(&["notes.md"]);
    let corpus = RootedCorpus::ambient(&ambient);
    let mounts = MountSet::default().with_unreachable(
        name("sessions"),
        "/Volumes/archive/sessions",
        "No such file or directory (os error 2)",
    );

    let got = CorpusIndex::new().resolve_ref("sessions:notes.md", "claim.md", &corpus, &mounts);
    let RefResolution::PathUnseeable { root, path, detail } = &got else {
        panic!("a DECLARED but unreadable root must not be Unmounted; got {got:?}");
    };
    assert_eq!(root, &name("sessions"));
    assert_eq!(
        path, "/Volumes/archive/sessions",
        "the refusal carries the PATH — the mount entry is already correct, so \
         naming the entry teaches nothing",
    );
    assert!(detail.contains("No such file or directory"));

    let teaching = model::selector::render_path_unseeable(root, path, detail);
    assert!(
        teaching.contains("/Volumes/archive/sessions"),
        "arm (b) names the PATH: {teaching}",
    );
    assert!(
        !teaching.contains("Fix: declare"),
        "arm (b) must NEVER prescribe the declaration — the root IS declared and \
         that fix is already done: {teaching}",
    );
}

/// The two arms must answer differently — asserted directly, since per-arm
/// checks would pass on a collapsed build too.
#[test]
fn the_two_root_greys_never_share_a_reason_word() {
    let ambient = corpus_with(&["notes.md"]);
    let corpus = RootedCorpus::ambient(&ambient);

    let unmounted =
        CorpusIndex::new().resolve_ref("sessions:x.md", "c.md", &corpus, &MountSet::default());
    let unseeable = CorpusIndex::new().resolve_ref(
        "sessions:x.md",
        "c.md",
        &corpus,
        &MountSet::default().with_unreachable(name("sessions"), "/gone", "unreadable"),
    );

    assert_ne!(
        unmounted, unseeable,
        "ONE address, TWO machine states, TWO answers — collapsing them is the \
         false negative this pair exists to prevent",
    );
    assert!(matches!(unmounted, RefResolution::Unmounted(_)));
    assert!(matches!(unseeable, RefResolution::PathUnseeable { .. }));
}

/// Acceptance half (S3-R8(c)): a declared, readable root still resolves into
/// its own corpus — else a build reporting every root unreadable would satisfy
/// both refusal tests above.
#[test]
fn a_declared_and_readable_root_still_resolves_into_its_own_corpus() {
    let ambient = corpus_with(&["notes.md"]);
    let sessions = corpus_with(&["notes.md", "deep/plan.md"]);
    let corpus =
        RootedCorpus::ambient(&ambient).with_root(name("sessions"), &sessions);
    let mounts = MountSet::new([name("sessions")]);

    let got = CorpusIndex::new().resolve_ref("sessions:deep/plan.md", "claim.md", &corpus, &mounts);
    assert_eq!(
        got,
        RefResolution::Rooted {
            root: name("sessions"),
            path: "deep/plan.md".to_owned(),
        },
        "bound and readable resolves INTO that root — grey is not the universal answer",
    );
}

#[test]
fn a_name_is_never_both_bound_and_unreachable() {
    let set = MountSet::new([name("sessions")]).with_unreachable(name("sessions"), "/gone", "why");
    assert!(
        !set.is_bound(&name("sessions")),
        "recording a name unreachable REMOVES it from the bound set",
    );
    assert!(set.unreachable(&name("sessions")).is_some());
    assert!(
        set.bound_names().is_empty(),
        "and `bound_names` stays truthful — a refusal that lists an unreadable \
         root among what is available teaches the wrong fix",
    );
}
