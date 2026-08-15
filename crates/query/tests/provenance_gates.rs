//! The §7 sql-provenance gates (merged plan §7, "sql row-provenance, both
//! directions" — codex gate 14's shape), run against the premise instrument
//! itself: emitted regions fold via the resident tree, entry vs live, and
//! the verdict is the fold compare. No journal exists anywhere in the loop —
//! the machinery's whole input surface is (sql text, tree).

use std::path::Path;

use fs::resident::{ResidentTree, ScopeFold};
use query::provenance::{SqlProvenance, classify};

fn h(seed: u8) -> [u8; 32] {
    [seed; 32]
}

/// Mint the premise set for a provenance answer: one `(scope, fold)` pair
/// per emitted scope — `fold_at` against the TREE, the same instrument as
/// every other guard.
fn mint(tree: &mut ResidentTree, provenance: &SqlProvenance) -> Vec<(String, ScopeFold)> {
    provenance
        .scopes()
        .into_iter()
        .map(|scope| {
            let fold = tree.fold_at(Path::new(&scope)).expect("scope resolves");
            (scope, fold)
        })
        .collect()
}

/// Check the premise set: every scope's live fold must still equal its
/// entry fold — the wire's `fingerprint_mismatch` is the false arm.
fn holds(tree: &mut ResidentTree, premises: &[(String, ScopeFold)]) -> bool {
    premises
        .iter()
        .all(|(scope, entry)| tree.fold_at(Path::new(scope)).expect("scope resolves") == *entry)
}

/// A corpus with a guarded subtree (`tasks/`) and foreign neighbors.
fn corpus() -> ResidentTree {
    let mut tree = ResidentTree::new();
    tree.set_leaf(Path::new("tasks/x.md"), h(1));
    tree.set_leaf(Path::new("tasks/y.md"), h(2));
    tree.set_leaf(Path::new("agents/y.md"), h(3));
    tree
}

/// §7 sql row, direction one: a path-provenance-bounded premise
/// (`path LIKE 'tasks/%'`) COMMITS on a foreign change outside `tasks/` —
/// the false conflict dies.
#[test]
fn path_bounded_premise_commits_on_foreign_change() {
    let mut tree = corpus();
    let provenance = classify("SELECT key, value FROM frontmatter WHERE path LIKE 'tasks/%'");
    assert_eq!(provenance.scopes(), vec!["tasks".to_owned()]);
    let premises = mint(&mut tree, &provenance);
    tree.set_leaf(Path::new("agents/y.md"), h(9));
    tree.set_leaf(Path::new("agents/new.md"), h(10));
    assert!(
        holds(&mut tree, &premises),
        "foreign churn outside tasks/ must not refuse a tasks/-bounded premise"
    );
}

/// §7 sql row, direction two: the same bounded premise REFUSES on a
/// matching before/after change inside `tasks/` — creation, modification,
/// deletion each move the subtree fold.
#[test]
fn path_bounded_premise_refuses_on_matching_change() {
    let provenance = classify("SELECT key, value FROM frontmatter WHERE path LIKE 'tasks/%'");
    for change in [
        (|t: &mut ResidentTree| {
            t.set_leaf(Path::new("tasks/x.md"), h(9));
        }) as fn(&mut ResidentTree),
        |t| {
            t.set_leaf(Path::new("tasks/new.md"), h(10));
        },
        |t| {
            t.remove_leaf(Path::new("tasks/y.md"));
        },
    ] {
        let mut tree = corpus();
        let premises = mint(&mut tree, &provenance);
        change(&mut tree);
        assert!(
            !holds(&mut tree, &premises),
            "a change inside tasks/ must refuse the tasks/-bounded premise"
        );
    }
}

/// §7 sql row, world arm — and the forced counterexample as the live demo
/// (card set-premises step 4): `backlink WHERE path = 'tasks/x.md'` is
/// path-constrained and still WORLD. A link written in `agents/y.md`
/// changes the query's result while `tasks/` never moves — the world
/// premise (the root) catches it; the path premise a WHERE-clause reading
/// would have minted stays green and would have committed a stale plan.
#[test]
fn world_provenance_conflicts_on_any_domain_change() {
    let mut tree = corpus();
    let provenance = classify("SELECT src_path FROM backlink WHERE path = 'tasks/x.md'");
    assert_eq!(provenance, SqlProvenance::World);
    let world = mint(&mut tree, &provenance);
    // The premise a WHERE-clause reading would have minted instead:
    let tasks_fold = tree.fold_at(Path::new("tasks")).expect("tasks resolves");

    // The new inbound link is a byte change in agents/y.md.
    tree.set_leaf(Path::new("agents/y.md"), h(9));

    assert!(
        !holds(&mut tree, &world),
        "world provenance must conflict on a domain change anywhere"
    );
    assert_eq!(
        tree.fold_at(Path::new("tasks")).expect("tasks resolves"),
        tasks_fold,
        "tasks/ never moved — exactly why the WHERE clause is not the law"
    );
}

/// The world premise also conflicts on changes far from any named path —
/// "any domain change", including a birth in a fresh directory.
#[test]
fn world_provenance_sees_every_corner() {
    let mut tree = corpus();
    let world = mint(&mut tree, &SqlProvenance::World);
    tree.set_leaf(Path::new("fresh/corner.md"), h(11));
    assert!(!holds(&mut tree, &world));
}

/// The no-journal assertion (card gate 3, structural half): premises are
/// minted and checked with ONLY the tree in hand — the signatures of
/// `classify`/`scopes`/`fold_at` admit no journal, no seq, no cursor. The
/// behavioral half: a tree built purely from leaves (no feed, no history)
/// serves every verdict above; this test re-runs a full mint-check cycle on
/// a fresh tree with zero event machinery in scope.
#[test]
fn no_premise_consults_the_journal() {
    let mut tree = ResidentTree::new();
    tree.set_leaf(Path::new("tasks/x.md"), h(1));
    let bounded = classify("SELECT * FROM doc WHERE path = 'tasks/x.md'");
    assert_eq!(bounded.scopes(), vec!["tasks/x.md".to_owned()]);
    let premises = mint(&mut tree, &bounded);
    assert!(holds(&mut tree, &premises));
    tree.set_leaf(Path::new("tasks/x.md"), h(2));
    assert!(!holds(&mut tree, &premises));
}
