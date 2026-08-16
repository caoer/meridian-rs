//! §5.4 scoped premises + §5.5 coverage at the splice door — the
//! fingerprint-grain family's engine substrate, exercised through the
//! in-process surface (the wire decode joins under the `scoped-guards` cap).
//!
//! The ruling's acceptance shape (decisions/2026-08-16-fp-grain-ruling):
//! a single-file put pinned on the FILE's own token survives a disjoint
//! sibling's birth, and a write that moves the planned scope still refuses,
//! naming the scope. Bare `if_fingerprint` stays the root premise (§5.1) —
//! its tests live in `u10_guard.rs` and `write_e2e` untouched.

use std::path::{Path, PathBuf};

use wire::{Edit, EditShape, ErrorBody, ErrorCode, HpathSeg, Path as WPath, SecRef};
use wire_serve::guard::{Origin, Premise, PremiseValue};
use wire_serve::write::{SpliceArgs, SpliceOutcome, scope_token, splice};

const DOC: &str = "# Target\n\nbody line one.\n";

/// A workspace with one file in a subtree of its own: `a/target.md`.
fn ws() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("a")).expect("mkdir");
    std::fs::write(dir.path().join("a/target.md"), DOC).expect("seed");
    let root = fs::WorkspaceRoot(PathBuf::from(dir.path()));
    (dir, root)
}

fn seg(h: &str) -> HpathSeg {
    HpathSeg {
        h: h.into(),
        n: None,
    }
}

/// One content edit on the Target section, deliberately UNGUARDED at node
/// grain — whether it is admitted is exactly what the premise list decides.
fn unguarded_edit() -> Edit {
    Edit {
        target: SecRef::Hpath {
            hpath: vec![seg("Target")],
        },
        edit: EditShape::Match {
            old: "body line one.".into(),
            new: "body line two.".into(),
        },
        if_node_rev: None,
    }
}

fn args(premises: Vec<Premise>) -> SpliceArgs {
    SpliceArgs {
        id: None,
        origin: Origin::Wire,
        path: WPath("a/target.md".into()),
        actor: Some("agent:test".into()),
        now: Some("2026-08-16T12:00:00Z".into()),
        receipt: None,
        if_root: None,
        premises,
        dry: false,
        force: false,
        edits: vec![unguarded_edit()],
        plan_edits: Vec::new(),
        pin: None,
    }
}

/// One in-process splice with no seq sink, no rulesets, no mint ledger.
fn run(root: &fs::WorkspaceRoot, a: &SpliceArgs) -> Result<SpliceOutcome, Box<ErrorBody>> {
    splice(root, None, a, &[], None)
}

/// The file's own §5.4 premise, minted through the one mint home.
fn file_premise(root: &fs::WorkspaceRoot) -> Premise {
    let token = scope_token(root, None, Some(Path::new("a/target.md")))
        .expect("mint")
        .expect("the seeded file holds a node");
    Premise {
        scope: Some(PathBuf::from("a/target.md")),
        value: PremiseValue::Token(token),
    }
}

// ── The ruling's acceptance shape ───────────────────────────────────────────

/// The dogfood shape at the engine: capture the FILE's token, birth a
/// DISJOINT sibling, put the untouched file pinned on that token — commits.
#[test]
fn a_file_premise_survives_a_disjoint_sibling_birth() {
    let (_dir, root) = ws();
    let premise = file_premise(&root);
    std::fs::create_dir_all(root.0.join("b")).expect("mkdir");
    std::fs::write(root.0.join("b/mover.md"), "# Mover\n\ndisjoint.\n").expect("birth");

    let outcome = run(&root, &args(vec![premise])).expect("the premise held");
    assert!(
        outcome.committed.is_some(),
        "the batch landed — a disjoint sibling must not refuse a file-scoped plan"
    );
    let bytes = std::fs::read_to_string(root.0.join("a/target.md")).expect("read back");
    assert!(bytes.contains("body line two."), "the edit reached disk");
}

/// The counter-proof: the same shape refuses when the planned scope ITSELF
/// moved — `fingerprint_mismatch` naming the scope (§5.7).
#[test]
fn a_file_premise_refuses_when_the_scope_itself_moved() {
    let (_dir, root) = ws();
    let premise = file_premise(&root);
    // The scope moves: an out-of-band append to the planned file. The match
    // target's own bytes stay intact, so only the premise can refuse.
    std::fs::write(
        root.0.join("a/target.md"),
        format!("{DOC}\nout-of-band tail.\n"),
    )
    .expect("move the scope");

    let err = run(&root, &args(vec![premise])).expect_err("the premise moved");
    assert_eq!(
        err.code,
        ErrorCode::RootMismatch,
        "wire: fingerprint_mismatch"
    );
    assert_eq!(
        err.scope.as_deref(),
        Some("a/target.md"),
        "a SCOPED mismatch names its premise (§5.7): {err:?}"
    );
    let (expected, actual) = (err.expected.expect("expected"), err.actual.expect("actual"));
    assert_ne!(expected.0, actual.0, "two different tokens, both spelled");
    let message = err.message.expect("teaching");
    assert!(
        message.contains("the premise at a/target.md moved"),
        "the §8.2 register text speaks: {message}"
    );
    assert!(
        message.contains("about this premise only"),
        "the refusal bounds its own claim: {message}"
    );
}

// ── §5.5 coverage at admission ──────────────────────────────────────────────

/// Premises present but none covering the edited file: `scope_does_not_cover`
/// naming the uncovered target set — never `guard_required` (A.1's split).
#[test]
fn an_uncovered_write_with_premises_refuses_scope_does_not_cover() {
    let (_dir, root) = ws();
    std::fs::create_dir_all(root.0.join("b")).expect("mkdir");
    std::fs::write(root.0.join("b/mover.md"), "# Mover\n\ndisjoint.\n").expect("birth");
    let elsewhere = Premise {
        scope: Some(PathBuf::from("b/mover.md")),
        value: PremiseValue::Token(
            scope_token(&root, None, Some(Path::new("b/mover.md")))
                .expect("mint")
                .expect("token"),
        ),
    };

    let err = run(&root, &args(vec![elsewhere])).expect_err("uncovered");
    assert_eq!(err.code, ErrorCode::ScopeDoesNotCover);
    let uncovered = err.uncovered.expect("the uncovered set is named");
    assert_eq!(uncovered, vec!["section \"Target\"".to_owned()]);
    let message = err.message.expect("teaching");
    assert!(
        message.contains("no premise covers them"),
        "the §8.2 register text speaks: {message}"
    );
}

/// NO premise at all keeps today's refusal: `guard_required`, byte-familiar —
/// A.1's exact meaning is preserved.
#[test]
fn no_premise_at_all_stays_guard_required() {
    let (_dir, root) = ws();
    let err = run(&root, &args(Vec::new())).expect_err("guardless");
    assert_eq!(err.code, ErrorCode::GuardRequired);
}

/// A folder premise covers every file under it — coverage is ancestor-or-self,
/// component-wise.
#[test]
fn a_folder_premise_covers_its_files() {
    let (_dir, root) = ws();
    let folder = Premise {
        scope: Some(PathBuf::from("a")),
        value: PremiseValue::Token(
            scope_token(&root, None, Some(Path::new("a")))
                .expect("mint")
                .expect("the folder holds a node"),
        ),
    };
    let outcome = run(&root, &args(vec![folder])).expect("covered and current");
    assert!(outcome.committed.is_some());
}

// ── `absent` — a value, not an error (§5.6) ────────────────────────────────

/// An absence premise holds while the path has no node, and refuses the
/// moment something is born there — the creation-guard plan's foundation.
#[test]
fn an_absence_premise_holds_until_the_path_is_born() {
    let (_dir, root) = ws();
    let absent = Premise {
        scope: Some(PathBuf::from("c/new.md")),
        value: PremiseValue::Absent,
    };
    let outcome =
        run(&root, &args(vec![absent.clone(), file_premise(&root)])).expect("absence holds");
    assert!(outcome.committed.is_some());

    std::fs::create_dir_all(root.0.join("c")).expect("mkdir");
    std::fs::write(root.0.join("c/new.md"), "# New\n").expect("birth");
    let err = run(&root, &args(vec![absent, file_premise(&root)])).expect_err("the path was born");
    assert_eq!(err.code, ErrorCode::RootMismatch);
    assert_eq!(err.scope.as_deref(), Some("c/new.md"));
    assert_eq!(err.expected.expect("expected").0, "absent");
}

/// The inverse: a token premise whose scope was emptied refuses with the
/// absent-actual teaching (§8.2) — `actual` is the reserved value.
#[test]
fn a_deleted_scope_refuses_with_the_absent_actual_teaching() {
    let (_dir, root) = ws();
    std::fs::create_dir_all(root.0.join("c")).expect("mkdir");
    std::fs::write(root.0.join("c/aux.md"), "# Aux\n").expect("seed");
    let aux = Premise {
        scope: Some(PathBuf::from("c/aux.md")),
        value: PremiseValue::Token(
            scope_token(&root, None, Some(Path::new("c/aux.md")))
                .expect("mint")
                .expect("token"),
        ),
    };
    std::fs::remove_file(root.0.join("c/aux.md")).expect("empty the scope");

    let err = run(&root, &args(vec![aux, file_premise(&root)])).expect_err("the scope was emptied");
    assert_eq!(err.code, ErrorCode::RootMismatch);
    assert_eq!(err.scope.as_deref(), Some("c/aux.md"));
    assert_eq!(err.actual.expect("actual").0, "absent");
    let message = err.message.expect("teaching");
    assert!(
        message.contains("has no node now"),
        "the absent-actual register text speaks: {message}"
    );
}

// ── The root premise as a list entry, and ordering ─────────────────────────

/// A root premise riding `premises` behaves as the v2 world guard: matches
/// the served root, and its mismatch carries NO scope field (§5.1 —
/// byte-identical shape).
#[test]
fn a_root_premise_entry_is_the_v2_world_guard() {
    let (_dir, root) = ws();
    let world = scope_token(&root, None, None).expect("mint").expect("root");
    let outcome = run(
        &root,
        &args(vec![Premise {
            scope: None,
            value: PremiseValue::Token(world),
        }]),
    )
    .expect("current world");
    assert!(outcome.committed.is_some());

    let (_dir2, root2) = ws();
    let err = run(
        &root2,
        &args(vec![Premise {
            scope: None,
            value: PremiseValue::Token(
                "b3a:0000000000000000000000000000000000000000000000000000000000000000".into(),
            ),
        }]),
    )
    .expect_err("stale world");
    assert_eq!(err.code, ErrorCode::RootMismatch);
    assert!(
        err.scope.is_none(),
        "the root premise refusal carries no scope"
    );
}

/// Widest first (merkle-spec §7): a failing ROOT premise answers before a
/// failing narrower one, whatever order the caller supplied.
#[test]
fn widest_first_a_failing_root_premise_answers_before_a_narrower_one() {
    let (_dir, root) = ws();
    let stale_file = Premise {
        scope: Some(PathBuf::from("a/target.md")),
        value: PremiseValue::Token(
            "b3a:1111111111111111111111111111111111111111111111111111111111111111".into(),
        ),
    };
    let stale_root = Premise {
        scope: None,
        value: PremiseValue::Token(
            "b3a:2222222222222222222222222222222222222222222222222222222222222222".into(),
        ),
    };
    let err = run(&root, &args(vec![stale_file, stale_root])).expect_err("both stale");
    assert!(
        err.scope.is_none(),
        "the root premise answered first: {err:?}"
    );
}

// ── `scope_unresolved` (§5.6) ───────────────────────────────────────────────

/// A scope escaping the workspace cannot hold a token — refused before any
/// probe outside the root.
#[test]
fn a_scope_that_escapes_the_workspace_refuses_scope_unresolved() {
    let (_dir, root) = ws();
    let escape = Premise {
        scope: Some(PathBuf::from("../outside.md")),
        value: PremiseValue::Absent,
    };
    let err = run(&root, &args(vec![escape, file_premise(&root)])).expect_err("escapes the root");
    assert_eq!(err.code, ErrorCode::ScopeUnresolved);
    let message = err.message.expect("teaching");
    assert!(
        message.contains("cannot hold a token"),
        "the §8.2 register text speaks: {message}"
    );
}

/// A scope that passes THROUGH an existing file conflicts in kind — the
/// tree's own `scope_unresolved` fact.
#[test]
fn a_kind_conflict_scope_refuses_scope_unresolved() {
    let (_dir, root) = ws();
    let conflicted = Premise {
        scope: Some(PathBuf::from("a/target.md/child.md")),
        value: PremiseValue::Absent,
    };
    let err = run(&root, &args(vec![conflicted, file_premise(&root)])).expect_err("kind conflict");
    assert_eq!(err.code, ErrorCode::ScopeUnresolved);
}

// ── The mint arm's grain, proven at the instrument ─────────────────────────

/// The grain fact itself: a disjoint birth moves the WORLD token and leaves
/// the FILE token unmoved — the exact asymmetry the dogfood exercises.
#[test]
fn a_disjoint_birth_moves_the_world_token_and_not_the_file_token() {
    let (_dir, root) = ws();
    let file_before = scope_token(&root, None, Some(Path::new("a/target.md")))
        .expect("mint")
        .expect("token");
    let world_before = scope_token(&root, None, None).expect("mint").expect("root");

    std::fs::create_dir_all(root.0.join("b")).expect("mkdir");
    std::fs::write(root.0.join("b/mover.md"), "# Mover\n").expect("birth");

    let file_after = scope_token(&root, None, Some(Path::new("a/target.md")))
        .expect("mint")
        .expect("token");
    let world_after = scope_token(&root, None, None).expect("mint").expect("root");

    assert_eq!(
        file_before, file_after,
        "the disjoint subtree's token stands"
    );
    assert_ne!(world_before, world_after, "the world token moved");
    assert_ne!(file_before, world_before, "two grains, two tokens");
}

// ── Malformed premise values — input, never a moved world ──────────────────

/// Dogfood break #7 (88877785): a valid mint token with ONE LEADING SPACE is
/// a damaged spelling, not a moved world. The refusal is `bad_request`
/// quoting the raw bytes so the space is visible — never a
/// `fingerprint_mismatch` whose expected/live render character-identical.
#[test]
fn a_spaced_token_refuses_as_bad_input_not_as_a_moved_premise() {
    let (_dir, root) = ws();
    let mut premise = file_premise(&root);
    let PremiseValue::Token(token) = premise.value.clone() else {
        panic!("file_premise mints a token");
    };
    let spaced = format!(" {token}");
    premise.value = PremiseValue::Token(spaced.clone());

    let err = run(&root, &args(vec![premise])).expect_err("a damaged spelling refuses");
    assert_eq!(
        err.code,
        ErrorCode::BadRequest,
        "an input defect, not a moved premise: {err:?}"
    );
    let message = err.message.expect("teaching");
    assert!(
        message.contains(&format!("{spaced:?}")),
        "the raw bytes are quoted so the space is visible: {message}"
    );
    assert!(
        !message.contains("moved"),
        "no moved-premise story for damaged input: {message}"
    );
}

/// The same law at the root grain: a spaced world token on the sugar slot
/// (`if_root`) is bad input, not a stale world.
#[test]
fn a_spaced_root_token_on_if_root_refuses_as_bad_input() {
    let (_dir, root) = ws();
    let world = scope_token(&root, None, None).expect("mint").expect("root");
    let mut a = args(Vec::new());
    a.if_root = Some(wire::Root(format!(" {world}")));

    let err = run(&root, &a).expect_err("a damaged spelling refuses");
    assert_eq!(err.code, ErrorCode::BadRequest, "{err:?}");
}

/// Garbage that never was a token (not `b3…:<64hex>`, not `absent`) is the
/// same input class — and the quoted spelling makes truncation visible.
#[test]
fn a_truncated_token_refuses_as_bad_input() {
    let (_dir, root) = ws();
    let premise = Premise {
        scope: Some(PathBuf::from("a/target.md")),
        value: PremiseValue::Token("b3c:7affb769".into()),
    };
    let err = run(&root, &args(vec![premise])).expect_err("not a token");
    assert_eq!(err.code, ErrorCode::BadRequest, "{err:?}");
    let message = err.message.expect("teaching");
    assert!(
        message.contains("\"b3c:7affb769\""),
        "the raw spelling is quoted: {message}"
    );
}
