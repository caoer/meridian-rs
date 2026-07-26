//! **U11 round 2 — the two ROOT greys are distinct causes with distinct words,
//! asserted at the resolver (S3-R43 / S3-R49).**
//!
//! # S3-R37 — this file IS the constructed population
//!
//! The declared-but-unreadable arm of `resolve_ref` is **unreachable through
//! `mrd walk`**: `walk_cmd::load_mounts` builds its `MountSet` and its corpora
//! from one pass, so a name it binds always has a corpus. The CLI therefore
//! **cannot** exhibit this arm, and a gate written only at the CLI would inherit
//! an empty population — the arm would compile, pass, and mean nothing.
//!
//! So the population is CONSTRUCTED here, the same way U8's `undeclared-probe`
//! root constructs one: a hand-built `MountSet` + `RootedCorpus` **pair that
//! disagree on purpose** — a name the table declares and the corpus does not
//! hold. That pairing is exactly what U12 and U13 will build the moment they
//! project a mount table independently of their corpora, which § 8 M6
//! contemplates. **The word must exist, and be exercised, before the units that
//! make the arm reachable are written.**
//!
//! # The redden, quoted
//!
//! Before the fix, on `9429a5fd`, both arms returned `RefResolution::Unmounted`,
//! so this file's `path_unseeable` assertions read:
//!
//! ```text
//! left: Unmounted(MountName("sessions"))
//! right: <a PathUnseeable carrying the path>
//! ```
//!
//! and the teaching rendered for both was *"names a root this machine does not
//! bind … Fix: declare 'sessions' in ~/MERIDIAN.md"* — **false on this arm, and
//! prescribing an action already taken.**

use addr::{MountName, MountSet};
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

/// **ARM (a) — a root NOTHING declares is `Unmounted`**, and its teaching is the
/// one that says *declare it*. That teaching is correct here and only here.
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

/// **ARM (b) — a root the table DECLARES but cannot read is `PathUnseeable`**,
/// carrying the PATH, and its teaching must never prescribe the declaration.
///
/// The `MountSet` and the `RootedCorpus` are built to DISAGREE: the table knows
/// `sessions`, the corpus holds nothing for it. That is the pairing production
/// cannot currently produce and U12/U13 will.
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

/// **The two arms do not share a word.** Asserted directly, because the whole
/// defect was that they did — and a test that checked each arm separately would
/// have passed on the collapsed build too.
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

/// **The ACCEPTANCE half (S3-R8(c)).** A declared root that IS readable still
/// resolves into its own corpus. Without this, a build that reported every root
/// as unreadable would satisfy both refusal tests above.
#[test]
fn a_declared_and_readable_root_still_resolves_into_its_own_corpus() {
    let ambient = corpus_with(&["notes.md"]);
    let sessions = corpus_with(&["notes.md", "deep/plan.md"]);
    let corpus =
        RootedCorpus::ambient(&ambient).with_root(name("sessions"), RootKind::Vault, &sessions);
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

/// **`is_bound` and `unreachable` are mutually exclusive**, so no caller can see
/// a name as both bound and unreadable and pick whichever it likes.
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
