//! The `md.create` birth cap (create-task-page ruling, 2026-08-18 — option 1:
//! engine birth cap for declared tasks). Births go through the CREATE DOOR:
//! occupied-path refusal (`cas_mismatch`), workspace confinement, checks —
//! the door's own guards, never re-implemented in the run plane.
//!
//! Path resolution (md-create-ambient-paths ruling, shape (c), 2026-08-18):
//! a BARE path resolves under the request's `ambient` (the caller's ambient
//! directory) when one rides, and stays workspace-root-relative when none
//! does — the bare-door law every `ambient: None` fixture here pins. A
//! rooted `root:rel` spelling is EXPLICIT (§4.1; the same-root/foreign table
//! legs live in `registry/tests/run_ambient.rs`, where the mount table is
//! controlled). Capability grain: `md.create` untargeted admits any path;
//! `md.create:<dir>` admits only births whose RESOLVED partition — the born
//! file's immediate parent directory — is `<dir>`; a root-level birth's
//! grain is the path itself.

use std::collections::BTreeMap;

use effects::{ArgValue, Effect, EffectKind, Provenance};
use model::MerkleRoot;
use run::caps::{Authority, CapSet};
use run::executor::{self, ApplyRequest, ExecError};

const PAGE: &str = "\
---
status: todo
---

# Tasks

## Log

- existing line
";

/// Empty run-birth fields (the CLI-entry shape; stamps are the wire arm's).
static EMPTY_FIELDS: BTreeMap<String, String> = BTreeMap::new();

fn workspace() -> (tempfile::TempDir, fs::WorkspaceRoot) {
    // Under target/, not $TMPDIR: macOS's /var→/private/var symlink makes the
    // door's cache canonicalization disagree with the fixture root (the same
    // host-env failure crates/check's e2e shows on mac at clean main; Linux
    // CI never sees it). A real path sidesteps the interplay.
    let tmp = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    std::fs::write(tmp.path().join("page.md"), PAGE).unwrap();
    let root = fs::WorkspaceRoot(tmp.path().to_owned());
    (tmp, root)
}

fn current_root(root: &fs::WorkspaceRoot) -> MerkleRoot {
    fs::domain_snapshot(root).unwrap().1
}

fn create_effect(path: &str, body: &str, seq: u32) -> Effect {
    Effect {
        kind: EffectKind::Create,
        rule_id: "t".to_owned(),
        seq,
        depth: 0,
        provenance: Provenance::Run {
            invocation_id: "inv-1".to_owned(),
            root_at_eval: "b3:x".to_owned(),
        },
        args: [
            ("path".to_owned(), ArgValue::Str(path.to_owned())),
            ("body".to_owned(), ArgValue::Str(body.to_owned())),
        ]
        .into_iter()
        .collect::<BTreeMap<_, _>>(),
    }
}

fn request<'a>(
    effects: &'a [Effect],
    authority: &'a Authority,
    observed: &'a MerkleRoot,
) -> ApplyRequest<'a> {
    ApplyRequest {
        page: "page.md",
        task: "t",
        task_rev: "cafecafecafecafe",
        invocation_id: "inv-1",
        now: None,
        effects,
        authority,
        observed_root: observed,
        receipt: None,
        exec: None,
        actor: None,
        depth: 0,
        delta: None,
        fields: &EMPTY_FIELDS,
        birth_seq: None,
        ambient: None,
    }
}

fn granted(caps: &str) -> Authority {
    Authority::granted(CapSet::parse(caps).unwrap())
}

/// The happy path: a granted `md.create` births the file through the door,
/// the born bytes are exactly the body, and the count includes the birth.
#[test]
fn a_granted_birth_lands_through_the_create_door() {
    let (tmp, root) = workspace();
    let effects = [create_effect("tasks/new-card.md", "# A card\n\nbody\n", 0)];
    let authority = granted("md.create");
    let observed = current_root(&root);
    let applied =
        executor::apply(&root, &request(&effects, &authority, &observed)).expect("the birth lands");
    assert_eq!(applied.applied, 1, "the birth counts as applied");
    let born = std::fs::read_to_string(tmp.path().join("tasks/new-card.md")).unwrap();
    assert!(
        born.contains("# A card"),
        "the born bytes carry the body: {born}"
    );
}

/// The occupied-path law: the same birth twice — the second refuses through
/// the door (`cas_mismatch`) and the file keeps its first bytes.
#[test]
fn an_occupied_path_refuses_the_second_birth() {
    let (tmp, root) = workspace();
    let effects = [create_effect("tasks/new-card.md", "# First\n", 0)];
    let authority = granted("md.create");
    let observed = current_root(&root);
    executor::apply(&root, &request(&effects, &authority, &observed)).expect("first birth lands");

    let again = [create_effect("tasks/new-card.md", "# Second\n", 0)];
    let observed = current_root(&root);
    let err = executor::apply(&root, &request(&again, &authority, &observed))
        .expect_err("an occupied path refuses");
    let ExecError::BirthRefused { path, detail } = err else {
        panic!("expected BirthRefused, got {err:?}");
    };
    assert_eq!(path, "tasks/new-card.md");
    assert!(
        detail.contains("cas_mismatch"),
        "the door's own refusal code rides the detail: {detail}"
    );
    let kept = std::fs::read_to_string(tmp.path().join("tasks/new-card.md")).unwrap();
    assert!(kept.contains("# First"), "the first bytes stand: {kept}");
}

/// Deny-by-default: no `md.create` grant — the choke point refuses before
/// any I/O and nothing is born.
#[test]
fn an_ungranted_birth_is_cap_denied_before_io() {
    let (tmp, root) = workspace();
    let effects = [create_effect("tasks/new-card.md", "# A card\n", 0)];
    let authority = granted("md.set_field");
    let observed = current_root(&root);
    let err = executor::apply(&root, &request(&effects, &authority, &observed))
        .expect_err("deny-by-default holds");
    assert!(
        matches!(err, ExecError::CapDenied { ref kind, .. } if kind == "md.create"),
        "the refusal names the kind: {err:?}"
    );
    assert!(
        !tmp.path().join("tasks/new-card.md").exists(),
        "nothing was born"
    );
}

/// The scoped grant: `md.create:tasks` admits a birth under `tasks/` and
/// refuses one under `notes/` — the capability grain is the resolved
/// partition (the born file's immediate parent directory), with the cap
/// grammar's exact-match semantics.
#[test]
fn a_scoped_grant_binds_the_resolved_partition_dir() {
    let (tmp, root) = workspace();
    let authority = granted("md.create:tasks");

    let inside = [create_effect("tasks/in-scope.md", "# In\n", 0)];
    let observed = current_root(&root);
    executor::apply(&root, &request(&inside, &authority, &observed))
        .expect("a tasks/ birth is admitted");
    assert!(tmp.path().join("tasks/in-scope.md").exists());

    let outside = [create_effect("notes/out-of-scope.md", "# Out\n", 0)];
    let observed = current_root(&root);
    let err = executor::apply(&root, &request(&outside, &authority, &observed))
        .expect_err("a notes/ birth is refused");
    assert!(
        matches!(err, ExecError::CapDenied { ref target, .. } if target == "notes"),
        "the refusal names the out-of-scope segment: {err:?}"
    );
    assert!(!tmp.path().join("notes/out-of-scope.md").exists());
}

/// A birth escaping the workspace refuses through the door's confinement.
#[test]
fn a_workspace_escaping_birth_refuses() {
    let (tmp, root) = workspace();
    let effects = [create_effect("../outside.md", "# Escape\n", 0)];
    let authority = granted("md.create");
    let observed = current_root(&root);
    let err = executor::apply(&root, &request(&effects, &authority, &observed))
        .expect_err("confinement holds");
    assert!(
        matches!(err, ExecError::BirthRefused { .. }),
        "the door refuses the escape: {err:?}"
    );
    assert!(!tmp.path().parent().unwrap().join("outside.md").exists());
}

/// A mixed generation: the birth realizes first, then the page batch lands —
/// one apply, both facts on disk.
#[test]
fn a_birth_and_a_page_edit_compose_in_one_generation() {
    let (tmp, root) = workspace();
    let effects = [
        create_effect("tasks/new-card.md", "# A card\n", 0),
        Effect {
            kind: EffectKind::SetField,
            rule_id: "t".to_owned(),
            seq: 1,
            depth: 0,
            provenance: Provenance::Run {
                invocation_id: "inv-1".to_owned(),
                root_at_eval: "b3:x".to_owned(),
            },
            args: [
                ("field".to_owned(), ArgValue::Str("status".to_owned())),
                ("value".to_owned(), ArgValue::Str("done".to_owned())),
            ]
            .into_iter()
            .collect::<BTreeMap<_, _>>(),
        },
    ];
    let authority = granted("md.create, md.set_field");
    let observed = current_root(&root);
    let applied = executor::apply(&root, &request(&effects, &authority, &observed))
        .expect("the generation lands");
    assert_eq!(applied.applied, 2, "both effects count");
    assert!(tmp.path().join("tasks/new-card.md").exists());
    let page = std::fs::read_to_string(tmp.path().join("page.md")).unwrap();
    assert!(
        page.contains("status: done"),
        "the page edit landed: {page}"
    );
}

/// A fixture request carrying the caller's ambient directory.
fn request_with_ambient<'a>(
    effects: &'a [Effect],
    authority: &'a Authority,
    observed: &'a MerkleRoot,
    ambient: &'a str,
) -> ApplyRequest<'a> {
    let mut req = request(effects, authority, observed);
    req.ambient = Some(ambient);
    req
}

/// The ambient lane (shape (c)): a bare birth path resolves under the
/// caller's ambient directory — the card lands on the CALLER's board, the
/// workspace root's stays empty, and `md.create:tasks` covers the resolved
/// partition wherever it lands.
#[test]
fn a_bare_birth_resolves_under_the_callers_ambient() {
    let (tmp, root) = workspace();
    let effects = [create_effect("tasks/card.md", "# A card\n", 0)];
    let authority = granted("md.create:tasks");
    let observed = current_root(&root);
    let ambient = "year=2026/month=08/18-00-adhoc";
    executor::apply(
        &root,
        &request_with_ambient(&effects, &authority, &observed, ambient),
    )
    .expect("the ambient birth lands");
    let born = tmp
        .path()
        .join("year=2026/month=08/18-00-adhoc/tasks/card.md");
    assert!(born.exists(), "the card lands on the caller's board");
    assert!(
        !tmp.path().join("tasks/card.md").exists(),
        "nothing lands at the workspace root — the original failure mode"
    );
}

/// Resolution precedes admission: the capability grain is judged on the
/// RESOLVED target, so the refusal names the resolved partition, never the
/// ambient's own first segment.
#[test]
fn the_capability_grain_is_judged_on_the_resolved_target() {
    let (tmp, root) = workspace();
    let effects = [create_effect("notes/card.md", "# A card\n", 0)];
    let authority = granted("md.create:tasks");
    let observed = current_root(&root);
    let ambient = "year=2026/month=08/18-00-adhoc";
    let err = executor::apply(
        &root,
        &request_with_ambient(&effects, &authority, &observed, ambient),
    )
    .expect_err("a notes/ partition refuses under md.create:tasks");
    assert!(
        matches!(err, ExecError::CapDenied { ref target, .. } if target == "notes"),
        "the refusal names the resolved partition: {err:?}"
    );
    assert!(!tmp.path().join(ambient).exists(), "nothing was born");
}

/// A bare path escaping THROUGH the ambient join refuses at the resolution
/// seam — before the capability grain and before any I/O.
#[test]
fn an_ambient_escaping_join_refuses() {
    let (tmp, root) = workspace();
    let effects = [create_effect("../../escape.md", "# Out\n", 0)];
    let authority = granted("md.create");
    let observed = current_root(&root);
    let err = executor::apply(
        &root,
        &request_with_ambient(
            &effects,
            &authority,
            &observed,
            "year=2026/month=08/18-00-adhoc",
        ),
    )
    .expect_err("the joined path is unconfined");
    assert!(
        matches!(err, ExecError::BirthRefused { .. }),
        "the resolution seam refuses the escape: {err:?}"
    );
    assert!(!tmp.path().parent().unwrap().join("escape.md").exists());
}

/// A malformed ambient refuses every birth it would have resolved — a host
/// defect surfaces loud, never as a silent root-relative landing.
#[test]
fn a_malformed_ambient_refuses_the_birth() {
    let (tmp, root) = workspace();
    let effects = [create_effect("tasks/card.md", "# A card\n", 0)];
    let authority = granted("md.create:tasks");
    let observed = current_root(&root);
    let err = executor::apply(
        &root,
        &request_with_ambient(&effects, &authority, &observed, "../outside"),
    )
    .expect_err("an unconfined ambient refuses");
    assert!(
        matches!(err, ExecError::BirthRefused { ref detail, .. }
            if detail.contains("ambient")),
        "the refusal names the ambient fault: {err:?}"
    );
    assert!(
        !tmp.path().join("tasks/card.md").exists(),
        "nothing was born"
    );
}

/// The §4.1 grammar walls that refuse BEFORE the mount table is ever read —
/// deterministic on any machine: two head colons, and a root with no path.
#[test]
fn rooted_grammar_faults_refuse_before_the_table() {
    let (_tmp, root) = workspace();
    let authority = granted("md.create");
    for bad in ["a:b:c.md", "sessions:"] {
        let effects = [create_effect(bad, "# X\n", 0)];
        let observed = current_root(&root);
        let err = executor::apply(&root, &request(&effects, &authority, &observed))
            .expect_err("a malformed rooted spelling refuses");
        assert!(
            matches!(err, ExecError::BirthRefused { .. }),
            "`{bad}` refuses through the resolution seam: {err:?}"
        );
    }
}
